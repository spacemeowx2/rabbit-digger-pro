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
    use tempfile::NamedTempFile;

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

    fn base_clash() -> Clash {
        Clash {
            rule_name: None,
            prefix: None,
            direct: None,
            reject: None,
            disable_proxy_group: false,
            select: None,
            name_map: BTreeMap::new(),
        }
    }

    #[test]
    fn test_get_target_defaults_and_errors() {
        let mut c = base_clash();
        c.name_map.insert("ProxyA".to_string(), "pA".to_string());

        assert_eq!(c.get_target("DIRECT").unwrap(), "local");
        assert_eq!(c.get_target("REJECT").unwrap(), "blackhole");
        assert_eq!(c.get_target("ProxyA").unwrap(), "pA");

        let err = c.get_target("Missing").unwrap_err();
        assert!(err.to_string().contains("Name not found"));
    }

    #[test]
    fn test_proxy_to_net_variants_and_plugin_errors() {
        let c = base_clash();

        let ss: Proxy = serde_json::from_value(serde_json::json!({
            "name": "p",
            "type": "ss",
            "server": "example.com",
            "port": 443,
            "cipher": "aes-128-gcm",
            "password": "pw",
            "udp": true
        }))
        .unwrap();
        assert_eq!(c.proxy_to_net(ss, None).unwrap().net_type, "shadowsocks");

        let ss_obfs: Proxy = serde_json::from_value(serde_json::json!({
            "name": "p",
            "type": "ss",
            "server": "example.com",
            "port": 443,
            "cipher": "aes-128-gcm",
            "password": "pw",
            "plugin": "obfs",
            "plugin-opts": {"mode": "tls"}
        }))
        .unwrap();
        assert_eq!(
            c.proxy_to_net(ss_obfs, None).unwrap().net_type,
            "shadowsocks"
        );

        let ss_bad_plugin: Proxy = serde_json::from_value(serde_json::json!({
            "name": "p",
            "type": "ss",
            "server": "example.com",
            "port": 443,
            "cipher": "aes-128-gcm",
            "password": "pw",
            "plugin": "something",
            "plugin-opts": {"mode": "tls"}
        }))
        .unwrap();
        assert!(c.proxy_to_net(ss_bad_plugin, None).is_err());

        let trojan: Proxy = serde_json::from_value(serde_json::json!({
            "name": "t",
            "type": "trojan",
            "server": "example.com",
            "port": 443,
            "password": "pw"
        }))
        .unwrap();
        assert_eq!(c.proxy_to_net(trojan, None).unwrap().net_type, "trojan");

        let http: Proxy = serde_json::from_value(serde_json::json!({
            "name": "h",
            "type": "http",
            "server": "example.com",
            "port": 8080
        }))
        .unwrap();
        assert_eq!(c.proxy_to_net(http, None).unwrap().net_type, "http");

        let socks5: Proxy = serde_json::from_value(serde_json::json!({
            "name": "s",
            "type": "socks5",
            "server": "example.com",
            "port": 1080
        }))
        .unwrap();
        assert_eq!(c.proxy_to_net(socks5, None).unwrap().net_type, "socks5");

        let other: Proxy = serde_json::from_value(serde_json::json!({
            "name": "x",
            "type": "unknown",
            "any": 1
        }))
        .unwrap();
        assert!(c.proxy_to_net(other, None).is_err());
    }

    #[test]
    fn test_proxy_group_to_net_select_and_relay_errors() {
        let mut c = base_clash();
        c.name_map.insert("a".to_string(), "a".to_string());
        c.name_map.insert("b".to_string(), "b".to_string());

        let mut proxy_map = HashMap::new();
        proxy_map.insert(
            "a".to_string(),
            serde_json::from_value(serde_json::json!({
                "name": "a",
                "type": "http",
                "server": "example.com",
                "port": 8080
            }))
            .unwrap(),
        );

        let pg_select = ProxyGroup {
            name: "g".to_string(),
            proxy_group_type: "select".to_string(),
            proxies: vec!["a".to_string(), "b".to_string()],
        };
        assert_eq!(
            c.proxy_group_to_net(pg_select, &proxy_map)
                .unwrap()
                .net_type,
            "select"
        );

        let pg_urltest = ProxyGroup {
            name: "g".to_string(),
            proxy_group_type: "url-test".to_string(),
            proxies: vec!["a".to_string()],
        };
        assert_eq!(
            c.proxy_group_to_net(pg_urltest, &proxy_map)
                .unwrap()
                .net_type,
            "select"
        );

        let pg_relay_missing_proxy = ProxyGroup {
            name: "g".to_string(),
            proxy_group_type: "relay".to_string(),
            proxies: vec!["b".to_string()],
        };
        assert!(c
            .proxy_group_to_net(pg_relay_missing_proxy, &proxy_map)
            .is_err());

        let pg_bad = ProxyGroup {
            name: "g".to_string(),
            proxy_group_type: "bad".to_string(),
            proxies: vec!["a".to_string()],
        };
        assert!(c.proxy_group_to_net(pg_bad, &proxy_map).is_err());
    }

    #[tokio::test]
    async fn test_rule_to_rule_variants_and_rule_set_file() {
        let mut c = base_clash();
        c.name_map.insert("ProxyA".to_string(), "pA".to_string());

        let cache = crate::storage::MemoryCache::new().await.unwrap();
        let oom_lock = Mutex::new(());

        let item = c
            .rule_to_rule(
                "DOMAIN-SUFFIX,example.com,ProxyA".to_string(),
                &cache,
                &BTreeMap::new(),
                &oom_lock,
            )
            .await
            .unwrap();
        match item.matcher {
            Matcher::Domain(d) => assert!(matches!(d.method, DomainMatcherMethod::Suffix)),
            _ => panic!("unexpected matcher"),
        }

        let item = c
            .rule_to_rule(
                "IP-CIDR,1.2.3.0/24,ProxyA".to_string(),
                &cache,
                &BTreeMap::new(),
                &oom_lock,
            )
            .await
            .unwrap();
        matches!(item.matcher, Matcher::IpCidr(_));

        let item = c
            .rule_to_rule(
                "SRC-IP-CIDR,10.0.0.0/8,ProxyA".to_string(),
                &cache,
                &BTreeMap::new(),
                &oom_lock,
            )
            .await
            .unwrap();
        matches!(item.matcher, Matcher::SrcIpCidr(_));

        let item = c
            .rule_to_rule(
                "MATCH,ProxyA".to_string(),
                &cache,
                &BTreeMap::new(),
                &oom_lock,
            )
            .await
            .unwrap();
        matches!(item.matcher, Matcher::Any(_));

        let item = c
            .rule_to_rule(
                "GEOIP,CN,ProxyA".to_string(),
                &cache,
                &BTreeMap::new(),
                &oom_lock,
            )
            .await
            .unwrap();
        matches!(item.matcher, Matcher::GeoIp(_));

        let mut rules = BTreeMap::new();
        let mut tmp = NamedTempFile::new().unwrap();
        std::io::Write::write_all(&mut tmp, b"payload:\n  - example.com\n  - foo.bar\n").unwrap();

        rules.insert(
            "set1".to_string(),
            RuleProvider {
                rule_type: "file".to_string(),
                behavior: "domain".to_string(),
                url: "".to_string(),
                path: tmp.path().to_string_lossy().to_string(),
                interval: 1,
            },
        );

        let item = c
            .rule_to_rule(
                "RULE-SET,set1,ProxyA".to_string(),
                &cache,
                &rules,
                &oom_lock,
            )
            .await
            .unwrap();

        match item.matcher {
            Matcher::Domain(d) => {
                assert!(matches!(d.method, DomainMatcherMethod::Match));
                assert!(d.domain.len() >= 2);
            }
            _ => panic!("unexpected matcher"),
        }
    }
}
