use std::{
    collections::{BTreeMap, HashMap},
    path::PathBuf,
    str::FromStr,
};

use anyhow::{anyhow, Result};
use futures::{stream, StreamExt};
use rabbit_digger::{
    config::{Config, Net},
    rd_std::rule::config::{
        self as rule_config, AnyMatcher, DomainMatcher, DomainMatcherMethod, GeoIpMatcher, IpCidr,
        IpCidrMatcher, Matcher, SrcIpCidrMatcher,
    },
};
use rd_interface::{
    async_trait, config::NetRef, prelude::*, rd_config, registry::Builder, IntoDyn,
};
use serde::Deserialize;
use serde_json::{json, Value};
use tokio::sync::Mutex;

use crate::{config::ImportSource, storage::Storage};

use super::{BoxImporter, Importer};

#[rd_config]
#[derive(Debug)]
pub struct Clash {
    rule_name: Option<String>,
    prefix: Option<String>,
    direct: Option<String>,
    reject: Option<String>,

    #[serde(default)]
    disable_proxy_group: bool,

    /// Make all proxies in the group name
    #[serde(default)]
    select: Option<String>,

    // reverse map from clash name to net name
    #[serde(skip)]
    name_map: BTreeMap<String, String>,
}

impl Builder<BoxImporter> for Clash {
    const NAME: &'static str = "clash";

    type Config = Clash;

    type Item = Clash;

    fn build(config: Self::Config) -> rd_interface::Result<Self::Item> {
        Ok(config)
    }
}

impl IntoDyn<BoxImporter> for Clash {
    fn into_dyn(self) -> BoxImporter {
        Box::new(self)
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "kebab-case")]
struct ClashConfig {
    proxies: Vec<Proxy>,
    proxy_groups: Vec<ProxyGroup>,
    rules: Vec<String>,

    #[serde(default)]
    rule_providers: BTreeMap<String, RuleProvider>,
}

#[derive(Debug, Deserialize, Clone)]
struct Proxy {
    name: String,
    #[serde(rename = "type")]
    proxy_type: String,
    #[serde(flatten)]
    opt: Value,
}

#[derive(Debug, Deserialize)]
struct ProxyGroup {
    name: String,
    #[serde(rename = "type")]
    proxy_group_type: String,
    proxies: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct RuleProvider {
    #[serde(rename = "type")]
    rule_type: String,
    behavior: String,
    url: String,
    path: String,
    interval: u64,
}

#[derive(Deserialize)]
struct RuleSet {
    payload: Vec<String>,
}

fn ghost_net() -> Net {
    Net::new(
        "alias",
        json!({
            "net": "noop"
        }),
    )
}

fn with_net(mut net: Net, target_net: Option<Net>) -> Net {
    if let (Some(obj), Some(target_net)) = (net.opt.as_object_mut(), target_net) {
        obj.insert("net".to_string(), serde_json::to_value(target_net).unwrap());
    }
    net
}

impl Clash {
    fn proxy_to_net(&self, p: Proxy, target_net: Option<Net>) -> Result<Net> {
        // TODO: http and socks5 has limited support
        let net: Net = match p.proxy_type.as_ref() {
            "ss" => {
                #[derive(Debug, Deserialize)]
                #[serde(rename_all = "kebab-case")]
                struct Param {
                    server: String,
                    port: u16,
                    cipher: String,
                    password: String,
                    udp: Option<bool>,
                    plugin: Option<String>,
                    plugin_opts: Option<HashMap<String, String>>,
                }
                let params: Param = serde_json::from_value(p.opt)?;

                if let (Some(plugin), Some(plugin_opts)) = (params.plugin, params.plugin_opts) {
                    if plugin == "obfs" {
                        let obfs_mode = plugin_opts
                            .get("mode")
                            .map(|i| i.to_string())
                            .unwrap_or_default();

                        let obfs_net = with_net(
                            Net::new(
                                "obfs",
                                json!({
                                    "obfs_mode": obfs_mode,
                                    "net": target_net,
                                }),
                            ),
                            target_net,
                        );

                        Net::new(
                            "shadowsocks",
                            json!({
                                "server": format!("{}:{}", params.server, params.port),
                                "cipher": params.cipher,
                                "password": params.password,
                                "udp": params.udp.unwrap_or_default(),
                                "net": obfs_net,
                            }),
                        )
                    } else {
                        return Err(anyhow!("unsupported plugin: {}", plugin));
                    }
                } else {
                    with_net(
                        Net::new(
                            "shadowsocks",
                            json!({
                                "server": format!("{}:{}", params.server, params.port),
                                "cipher": params.cipher,
                                "password": params.password,
                                "udp": params.udp.unwrap_or_default(),
                            }),
                        ),
                        target_net,
                    )
                }
            }
            "trojan" => {
                #[derive(Debug, Deserialize)]
                struct Param {
                    server: String,
                    port: u16,
                    password: String,
                    // udp is ignored
                    // udp: Option<bool>,
                    sni: Option<String>,
                    #[serde(rename = "skip-cert-verify")]
                    skip_cert_verify: Option<bool>,
                }
                let params: Param = serde_json::from_value(p.opt)?;
                with_net(
                    Net::new(
                        "trojan",
                        json!({
                            "server": format!("{}:{}", params.server, params.port),
                            "password": params.password,
                            "sni": params.sni.unwrap_or(params.server),
                            "skip_cert_verify": params.skip_cert_verify.unwrap_or_default(),
                        }),
                    ),
                    target_net,
                )
            }
            "http" => {
                #[derive(Debug, Deserialize)]
                struct Param {
                    server: String,
                    port: u16,
                }
                let params: Param = serde_json::from_value(p.opt)?;
                with_net(
                    Net::new(
                        "http",
                        json!({
                            "server": format!("{}:{}", params.server, params.port),
                        }),
                    ),
                    target_net,
                )
            }
            "socks5" => {
                #[derive(Debug, Deserialize)]
                struct Param {
                    server: String,
                    port: u16,
                }
                let params: Param = serde_json::from_value(p.opt)?;
                with_net(
                    Net::new(
                        "socks5",
                        json!({
                            "server": format!("{}:{}", params.server, params.port),
                        }),
                    ),
                    target_net,
                )
            }
            _ => return Err(anyhow!("Unsupported proxy type: {}", p.proxy_type)),
        };
        Ok(net)
    }

    fn get_target(&self, target: &str) -> Result<String> {
        if target == "DIRECT" {
            return Ok(self.direct.clone().unwrap_or_else(|| "local".to_string()));
        }
        if target == "REJECT" {
            return Ok(self
                .reject
                .clone()
                .unwrap_or_else(|| "blackhole".to_string()));
        }
        let net_name = self.name_map.get(target);
        net_name
            .map(|i| i.to_string())
            .ok_or_else(|| anyhow!("Name not found. clash name: {}", target))
    }

    fn proxy_group_to_net(&self, p: ProxyGroup, proxy_map: &HashMap<String, Proxy>) -> Result<Net> {
        let net_list = p
            .proxies
            .into_iter()
            .map(|name| self.get_target(&name))
            .collect::<Result<Vec<String>>>()?;
        let proxy_group_type = p.proxy_group_type.as_ref();

        Ok(match proxy_group_type {
            "select" => Net::new(
                "select",
                json!({
                    "selected": net_list.get(0).cloned().unwrap_or_else(|| "noop".to_string()),
                    "list": net_list,
                }),
            ),
            "url-test" | "fallback" => {
                tracing::warn!(
                    "Unsupported proxy group type: {}, will use select as fallback.",
                    proxy_group_type
                );
                Net::new(
                    "select",
                    json!({
                        "selected": net_list.get(0).cloned().unwrap_or_else(|| "noop".to_string()),
                        "list": net_list,
                    }),
                )
            }
            "relay" => {
                let net = net_list.iter().try_fold(
                    Net::new(
                        "alias",
                        json!({
                            "net": "local"
                        }),
                    ),
                    |acc, x| {
                        let proxy = proxy_map.get(x).ok_or(anyhow!(
                            "proxy {} not found in proxy group {}",
                            x,
                            p.name
                        ))?;

                        self.proxy_to_net(proxy.clone(), Some(acc))
                    },
                )?;

                net
            }
            _ => {
                return Err(anyhow!(
                    "Unsupported proxy group type: {}",
                    p.proxy_group_type
                ))
            }
        })
    }

    async fn rule_to_rule(
        &self,
        r: String,
        cache: &dyn Storage,
        rule_providers: &BTreeMap<String, RuleProvider>,
        oom_lock: &Mutex<()>,
    ) -> Result<rule_config::RuleItem> {
        let bad_rule = || anyhow!("Bad rule.");
        let mut ps = r.split(',');
        let mut ps_next = || ps.next().ok_or_else(bad_rule);
        let rule_type = ps_next()?;
        let item = match rule_type {
            "DOMAIN-SUFFIX" | "DOMAIN-KEYWORD" | "DOMAIN" => {
                let domain = ps_next()?.to_string();
                let target = NetRef::new(self.get_target(ps_next()?)?.into());
                let method = match rule_type {
                    "DOMAIN-SUFFIX" => DomainMatcherMethod::Suffix,
                    "DOMAIN-KEYWORD" => DomainMatcherMethod::Keyword,
                    "DOMAIN" => DomainMatcherMethod::Match,
                    _ => return Err(bad_rule()),
                };
                rule_config::RuleItem {
                    target,
                    matcher: Matcher::Domain(DomainMatcher {
                        method,
                        domain: domain.into(),
                    }),
                }
            }
            "IP-CIDR" | "IP-CIDR6" => {
                let ip_cidr = ps_next()?.to_string();
                let target = NetRef::new(self.get_target(ps_next()?)?.into());
                rule_config::RuleItem {
                    target,
                    matcher: Matcher::IpCidr(IpCidrMatcher {
                        ipcidr: IpCidr::from_str(&ip_cidr)?.into(),
                    }),
                }
            }
            "SRC-IP-CIDR" => {
                let ip_cidr = ps_next()?.to_string();
                let target = NetRef::new(self.get_target(ps_next()?)?.into());
                rule_config::RuleItem {
                    target,
                    matcher: Matcher::SrcIpCidr(SrcIpCidrMatcher {
                        ipcidr: IpCidr::from_str(&ip_cidr)?.into(),
                    }),
                }
            }
            "MATCH" => {
                let target = NetRef::new(self.get_target(ps_next()?)?.into());
                rule_config::RuleItem {
                    target,
                    matcher: Matcher::Any(AnyMatcher {}),
                }
            }
            "GEOIP" => {
                let region = ps_next()?.to_string();
                let target = NetRef::new(self.get_target(ps_next()?)?.into());
                rule_config::RuleItem {
                    target,
                    matcher: Matcher::GeoIp(GeoIpMatcher { country: region }),
                }
            }
            "RULE-SET" => {
                let set = ps_next()?.to_string();
                let target = NetRef::new(self.get_target(ps_next()?)?.into());
                let rule_provider = rule_providers.get(&set).ok_or_else(bad_rule)?;

                let source = match rule_provider.rule_type.as_ref() {
                    "http" => ImportSource::new_poll(
                        rule_provider.url.to_string(),
                        Some(rule_provider.interval),
                    ),
                    "file" => ImportSource::new_path(PathBuf::from(rule_provider.path.to_string())),
                    _ => return Err(bad_rule()),
                };

                let source_str = source.get_content(cache).await?;
                let _guard = oom_lock.lock().await;

                let RuleSet { payload } = serde_yaml::from_str(&source_str)?;
                match rule_provider.behavior.as_ref() {
                    "domain" => rule_config::RuleItem {
                        target: target.clone(),
                        matcher: Matcher::Domain(DomainMatcher {
                            method: DomainMatcherMethod::Match,
                            domain: payload.into(),
                        }),
                    },
                    "ipcidr" => rule_config::RuleItem {
                        target: target.clone(),
                        matcher: Matcher::IpCidr(IpCidrMatcher {
                            ipcidr: payload
                                .into_iter()
                                .map(|i| IpCidr::from_str(&i))
                                .collect::<rd_interface::Result<Vec<_>>>()?
                                .into(),
                        }),
                    },
                    // TODO: support classical behavior
                    _ => return Err(bad_rule()),
                }
            }
            _ => return Err(anyhow!("Rule prefix {} is not supported", rule_type)),
        };

        Ok(item)
    }

    fn proxy_group_name(&self, pg: impl AsRef<str>) -> String {
        self.prefix(pg)
    }

    fn prefix(&self, s: impl AsRef<str>) -> String {
        match &self.prefix {
            Some(prefix) => format!("{}.{}", prefix, s.as_ref()),
            None => s.as_ref().to_string(),
        }
    }
}

#[async_trait]
impl Importer for Clash {
    async fn process(
        &mut self,
        config: &mut Config,
        content: &str,
        cache: &dyn Storage,
    ) -> Result<()> {
        let clash_config: ClashConfig = serde_yaml::from_str(content)?;
        let mut added_proxies = Vec::new();
        let mut proxy_map = HashMap::new();

        for p in clash_config.proxies {
            let old_name = p.name.clone();
            let name = self.prefix(&old_name);
            added_proxies.push(name.clone());
            self.name_map.insert(old_name.clone(), name.clone());
            proxy_map.insert(old_name.clone(), p.clone());
            match self.proxy_to_net(p, None) {
                Ok(p) => {
                    config.net.insert(name, p);
                }
                Err(e) => {
                    tracing::warn!("proxy {} not translated: {:?}", old_name, e);
                    config.net.insert(name, ghost_net());
                }
            };
        }

        if !self.disable_proxy_group {
            for old_name in clash_config.proxy_groups.iter().map(|i| &i.name) {
                let name = self.proxy_group_name(old_name);
                self.name_map.insert(old_name.clone(), name.clone());
            }

            let proxy_groups = clash_config
                .proxy_groups
                .into_iter()
                .map(|i| (i.name.to_string(), i))
                .collect::<Vec<_>>();

            for (old_name, pg) in proxy_groups {
                let name = self.proxy_group_name(&old_name);
                match self.proxy_group_to_net(pg, &proxy_map) {
                    Ok(pg) => {
                        config.net.insert(name, pg);
                    }
                    Err(e) => {
                        tracing::warn!("proxy_group {} not translated: {:?}", old_name, e);
                    }
                };
            }
        }

        if let Some(rule_name) = &self.rule_name {
            let oom_lock = Mutex::new(());
            let rule = stream::iter(clash_config.rules)
                .map(|r| self.rule_to_rule(r, cache, &clash_config.rule_providers, &oom_lock))
                .buffered(10)
                .flat_map(stream::iter)
                .fold(
                    Vec::<rule_config::RuleItem>::new(),
                    |mut state, item| async move {
                        let mut merged = false;
                        if let Some(last) = state.last_mut() {
                            merged = last.merge(&item);
                        }
                        if !merged {
                            state.push(item);
                        }
                        state
                    },
                )
                .await;

            config
                .net
                .insert(rule_name.clone(), Net::new("rule", json!({ "rule": rule })));
        }

        if let Some(select) = &self.select {
            config.net.insert(
                select.clone(),
                Net::new(
                    "select",
                    json!({
                        "selected": added_proxies.get(0).cloned().unwrap_or_else(|| "noop".to_string()),
                        "list": added_proxies,
                    }),
                ),
            );
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_yaml::from_str;
    use std::fs;

    #[tokio::test]
    async fn test_importer_clash_relay() {
        let mut clash = Clash {
            rule_name: None,
            prefix: None,
            direct: None,
            reject: None,
            disable_proxy_group: false,
            select: None,
            name_map: BTreeMap::new(),
        };

        let content = fs::read_to_string("tests/relay_clash.yml").expect("Unable to read file");
        let wanted_content =
            fs::read_to_string("tests/relay_rdp.yml").expect("Unable to read file");

        let mut config = from_str::<Config>(&content).unwrap();
        let cache = crate::storage::MemoryCache::new().await.unwrap();
        clash.process(&mut config, &content, &cache).await.unwrap();

        let config_string = serde_yaml::to_string(&config).unwrap();

        assert_eq!(config_string, wanted_content);
    }

    #[test]
    fn test_clash_builder() {
        let result = Clash::build(Clash {
            rule_name: Some("test".to_string()),
            prefix: Some("prefix_".to_string()),
            direct: Some("direct".to_string()),
            reject: Some("reject".to_string()),
            disable_proxy_group: false,
            select: Some("select_".to_string()),
            name_map: std::collections::BTreeMap::new(),
        });
        assert!(result.is_ok());
    }

    #[test]
    fn test_clash_builder_default() {
        let result = Clash::build(Clash {
            rule_name: None,
            prefix: None,
            direct: None,
            reject: None,
            disable_proxy_group: false,
            select: None,
            name_map: std::collections::BTreeMap::new(),
        });
        assert!(result.is_ok());
    }

    #[test]
    fn test_clash_into_dyn() {
        let clash = Clash::build(Clash {
            rule_name: None,
            prefix: None,
            direct: None,
            reject: None,
            disable_proxy_group: false,
            select: None,
            name_map: std::collections::BTreeMap::new(),
        })
        .unwrap();
        let _ = clash.into_dyn();
    }
}
