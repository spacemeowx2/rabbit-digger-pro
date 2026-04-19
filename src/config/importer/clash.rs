use std::{
    collections::{BTreeMap, HashMap},
    path::PathBuf,
    str::FromStr,
};

use anyhow::{anyhow, Result};
use futures::{future::BoxFuture, stream, StreamExt, TryStreamExt};
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
#[serde(untagged)]
enum PortValue {
    Number(u16),
    String(String),
}

impl PortValue {
    fn into_u16(self) -> Result<u16> {
        match self {
            Self::Number(port) => Ok(port),
            Self::String(port) => port
                .parse()
                .map_err(|e| anyhow!("invalid port {port}: {e}")),
        }
    }
}

#[derive(Debug, Deserialize)]
struct ProxyGroup {
    name: String,
    #[serde(rename = "type")]
    proxy_group_type: String,
    proxies: Vec<String>,
    url: Option<String>,
    interval: Option<u64>,
    tolerance: Option<u64>,
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
                    port: PortValue,
                    cipher: String,
                    password: String,
                    udp: Option<bool>,
                    plugin: Option<String>,
                    plugin_opts: Option<HashMap<String, String>>,
                }
                let params: Param = serde_json::from_value(p.opt)?;
                let port = params.port.into_u16()?;

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
                                "server": format!("{}:{}", params.server, port),
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
                                "server": format!("{}:{}", params.server, port),
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
                #[serde(rename_all = "kebab-case")]
                struct Param {
                    server: String,
                    port: PortValue,
                    password: String,
                    // udp is ignored
                    // udp: Option<bool>,
                    sni: Option<String>,
                    #[serde(rename = "skip-cert-verify")]
                    skip_cert_verify: Option<bool>,
                }
                let params: Param = serde_json::from_value(p.opt)?;
                let port = params.port.into_u16()?;
                with_net(
                    Net::new(
                        "trojan",
                        json!({
                            "server": format!("{}:{}", params.server, port),
                            "password": params.password,
                            "sni": params.sni.unwrap_or(params.server),
                            "skip_cert_verify": params.skip_cert_verify.unwrap_or_default(),
                        }),
                    ),
                    target_net,
                )
            }
            "vless" => {
                #[derive(Debug, Deserialize)]
                #[serde(rename_all = "kebab-case")]
                struct RealityOpts {
                    public_key: String,
                    short_id: Option<String>,
                }

                #[derive(Debug, Deserialize)]
                #[serde(rename_all = "kebab-case")]
                struct Param {
                    server: String,
                    port: PortValue,
                    uuid: String,
                    udp: Option<bool>,
                    network: Option<String>,
                    flow: Option<String>,
                    tls: Option<bool>,
                    servername: Option<String>,
                    sni: Option<String>,
                    skip_cert_verify: Option<bool>,
                    client_fingerprint: Option<String>,
                    reality_opts: Option<RealityOpts>,
                }
                let params: Param = serde_json::from_value(p.opt)?;
                let port = params.port.into_u16()?;

                if let Some(network) = params.network.as_deref() {
                    if !network.eq_ignore_ascii_case("tcp") {
                        return Err(anyhow!("unsupported vless network: {network}"));
                    }
                }
                if matches!(params.tls, Some(false)) {
                    return Err(anyhow!("vless without tls is not supported"));
                }

                let sni = params
                    .servername
                    .or(params.sni)
                    .unwrap_or_else(|| params.server.clone());

                let mut opt = json!({
                    "server": format!("{}:{}", params.server, port),
                    "id": params.uuid,
                    "flow": params.flow,
                    "sni": sni,
                    "skip_cert_verify": params.skip_cert_verify.unwrap_or_default(),
                    "udp": params.udp.unwrap_or_default(),
                });
                if let Some(client_fingerprint) = params.client_fingerprint {
                    opt["client_fingerprint"] = json!(client_fingerprint);
                }
                if let Some(reality_opts) = params.reality_opts {
                    opt["reality_public_key"] = json!(reality_opts.public_key);
                    opt["reality_short_id"] = json!(reality_opts.short_id);
                }

                with_net(Net::new("vless", opt), target_net)
            }
            "http" => {
                #[derive(Debug, Deserialize)]
                struct Param {
                    server: String,
                    port: PortValue,
                }
                let params: Param = serde_json::from_value(p.opt)?;
                let port = params.port.into_u16()?;
                with_net(
                    Net::new(
                        "http",
                        json!({
                            "server": format!("{}:{}", params.server, port),
                        }),
                    ),
                    target_net,
                )
            }
            "socks5" => {
                #[derive(Debug, Deserialize)]
                struct Param {
                    server: String,
                    port: PortValue,
                }
                let params: Param = serde_json::from_value(p.opt)?;
                let port = params.port.into_u16()?;
                with_net(
                    Net::new(
                        "socks5",
                        json!({
                            "server": format!("{}:{}", params.server, port),
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
            "url-test" => Net::new(
                "url-test",
                json!({
                    "selected": net_list.get(0).cloned().unwrap_or_else(|| "noop".to_string()),
                    "list": net_list,
                    "url": p.url.unwrap_or_else(|| "http://www.gstatic.com/generate_204".to_string()),
                    "interval": p.interval.unwrap_or(300),
                    "tolerance": p.tolerance.unwrap_or(0),
                }),
            ),
            "fallback" => Net::new(
                "fallback",
                json!({
                    "selected": net_list.get(0).cloned().unwrap_or_else(|| "noop".to_string()),
                    "list": net_list,
                    "url": p.url.unwrap_or_else(|| "http://www.gstatic.com/generate_204".to_string()),
                    "interval": p.interval.unwrap_or(300),
                }),
            ),
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
    ) -> Result<Vec<rule_config::RuleItem>> {
        self.rule_to_rule_with_target(r, None, cache, rule_providers, oom_lock)
            .await
    }

    fn rule_to_rule_with_target<'a>(
        &'a self,
        r: String,
        inherited_target: Option<NetRef>,
        cache: &'a dyn Storage,
        rule_providers: &'a BTreeMap<String, RuleProvider>,
        oom_lock: &'a Mutex<()>,
    ) -> BoxFuture<'a, Result<Vec<rule_config::RuleItem>>> {
        Box::pin(async move {
            let bad_rule = || anyhow!("Bad rule.");
            let mut ps = r.split(',');
            let mut ps_next = || ps.next().ok_or_else(bad_rule);
            let rule_type = ps_next()?;
            let items = match rule_type {
                "DOMAIN-SUFFIX" | "DOMAIN-KEYWORD" | "DOMAIN" => {
                    let domain = ps_next()?.to_string();
                    let target = match inherited_target.clone() {
                        Some(target) => target,
                        None => NetRef::new(self.get_target(ps_next()?)?.into()),
                    };
                    let method = match rule_type {
                        "DOMAIN-SUFFIX" => DomainMatcherMethod::Suffix,
                        "DOMAIN-KEYWORD" => DomainMatcherMethod::Keyword,
                        "DOMAIN" => DomainMatcherMethod::Match,
                        _ => return Err(bad_rule()),
                    };
                    vec![rule_config::RuleItem {
                        target,
                        matcher: Matcher::Domain(DomainMatcher {
                            method,
                            domain: domain.into(),
                        }),
                    }]
                }
                "IP-CIDR" | "IP-CIDR6" => {
                    let ip_cidr = ps_next()?.to_string();
                    let target = match inherited_target.clone() {
                        Some(target) => target,
                        None => NetRef::new(self.get_target(ps_next()?)?.into()),
                    };
                    vec![rule_config::RuleItem {
                        target,
                        matcher: Matcher::IpCidr(IpCidrMatcher {
                            ipcidr: IpCidr::from_str(&ip_cidr)?.into(),
                        }),
                    }]
                }
                "SRC-IP-CIDR" => {
                    let ip_cidr = ps_next()?.to_string();
                    let target = match inherited_target.clone() {
                        Some(target) => target,
                        None => NetRef::new(self.get_target(ps_next()?)?.into()),
                    };
                    vec![rule_config::RuleItem {
                        target,
                        matcher: Matcher::SrcIpCidr(SrcIpCidrMatcher {
                            ipcidr: IpCidr::from_str(&ip_cidr)?.into(),
                        }),
                    }]
                }
                "MATCH" => {
                    let target = match inherited_target.clone() {
                        Some(target) => target,
                        None => NetRef::new(self.get_target(ps_next()?)?.into()),
                    };
                    vec![rule_config::RuleItem {
                        target,
                        matcher: Matcher::Any(AnyMatcher {}),
                    }]
                }
                "GEOIP" => {
                    let region = ps_next()?.to_string();
                    let target = match inherited_target.clone() {
                        Some(target) => target,
                        None => NetRef::new(self.get_target(ps_next()?)?.into()),
                    };
                    vec![rule_config::RuleItem {
                        target,
                        matcher: Matcher::GeoIp(GeoIpMatcher { country: region }),
                    }]
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
                        "file" => {
                            ImportSource::new_path(PathBuf::from(rule_provider.path.to_string()))
                        }
                        _ => return Err(bad_rule()),
                    };

                    let source_str = source.get_content(cache).await?;
                    let _guard = oom_lock.lock().await;

                    let RuleSet { payload } = serde_yaml::from_str(&source_str)?;
                    match rule_provider.behavior.as_ref() {
                        "domain" => vec![rule_config::RuleItem {
                            target: target.clone(),
                            matcher: Matcher::Domain(DomainMatcher {
                                method: DomainMatcherMethod::Match,
                                domain: payload.into(),
                            }),
                        }],
                        "ipcidr" => vec![rule_config::RuleItem {
                            target: target.clone(),
                            matcher: Matcher::IpCidr(IpCidrMatcher {
                                ipcidr: payload
                                    .into_iter()
                                    .map(|i| IpCidr::from_str(&i))
                                    .collect::<rd_interface::Result<Vec<_>>>()?
                                    .into(),
                            }),
                        }],
                        "classical" => {
                            let mut items = Vec::new();
                            for rule in payload {
                                items.extend(
                                    self.rule_to_rule_with_target(
                                        rule,
                                        Some(target.clone()),
                                        cache,
                                        rule_providers,
                                        oom_lock,
                                    )
                                    .await?,
                                );
                            }
                            items
                        }
                        _ => return Err(bad_rule()),
                    }
                }
                _ => return Err(anyhow!("Rule prefix {} is not supported", rule_type)),
            };

            Ok(items)
        })
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
                .try_fold(
                    Vec::<rule_config::RuleItem>::new(),
                    |mut state, items| async move {
                        for item in items {
                            let mut merged = false;
                            if let Some(last) = state.last_mut() {
                                merged = last.merge(&item);
                            }
                            if !merged {
                                state.push(item);
                            }
                        }
                        Ok(state)
                    },
                )
                .await?;

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
            "port": "443",
            "password": "pw"
        }))
        .unwrap();
        let trojan = c.proxy_to_net(trojan, None).unwrap();
        assert_eq!(trojan.net_type, "trojan");
        assert_eq!(
            trojan.opt.get("server").and_then(|v| v.as_str()),
            Some("example.com:443")
        );

        let vless: Proxy = serde_json::from_value(serde_json::json!({
            "name": "v",
            "type": "vless",
            "server": "example.com",
            "port": 443,
            "uuid": "718735b0-4a1a-3663-a190-a84fcd981921",
            "udp": true,
            "network": "tcp",
            "flow": "xtls-rprx-vision",
            "tls": true,
            "servername": "www.microsoft.com"
        }))
        .unwrap();
        let vless = c.proxy_to_net(vless, None).unwrap();
        assert_eq!(vless.net_type, "vless");
        assert_eq!(
            vless.opt.get("server").and_then(|v| v.as_str()),
            Some("example.com:443")
        );
        assert_eq!(
            vless.opt.get("id").and_then(|v| v.as_str()),
            Some("718735b0-4a1a-3663-a190-a84fcd981921")
        );
        assert_eq!(
            vless.opt.get("flow").and_then(|v| v.as_str()),
            Some("xtls-rprx-vision")
        );
        assert_eq!(
            vless.opt.get("sni").and_then(|v| v.as_str()),
            Some("www.microsoft.com")
        );
        assert!(vless.opt.get("reality_public_key").is_none());

        let vless_ws: Proxy = serde_json::from_value(serde_json::json!({
            "name": "vw",
            "type": "vless",
            "server": "example.com",
            "port": 443,
            "uuid": "718735b0-4a1a-3663-a190-a84fcd981921",
            "network": "ws",
            "tls": true
        }))
        .unwrap();
        assert!(c.proxy_to_net(vless_ws, None).is_err());

        let vless_reality: Proxy = serde_json::from_value(serde_json::json!({
            "name": "vr",
            "type": "vless",
            "server": "example.com",
            "port": 29582,
            "uuid": "718735b0-4a1a-3663-a190-a84fcd981921",
            "udp": true,
            "network": "tcp",
            "flow": "xtls-rprx-vision",
            "tls": true,
            "servername": "www.microsoft.com",
            "client-fingerprint": "chrome",
            "reality-opts": {
                "public-key": "QUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUE",
                "short-id": "00000000"
            }
        }))
        .unwrap();
        let vless_reality = c.proxy_to_net(vless_reality, None).unwrap();
        assert_eq!(
            vless_reality
                .opt
                .get("client_fingerprint")
                .and_then(|v| v.as_str()),
            Some("chrome")
        );
        assert_eq!(
            vless_reality
                .opt
                .get("reality_public_key")
                .and_then(|v| v.as_str()),
            Some("QUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUE")
        );
        assert_eq!(
            vless_reality
                .opt
                .get("reality_short_id")
                .and_then(|v| v.as_str()),
            Some("00000000")
        );

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
            url: None,
            interval: None,
            tolerance: None,
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
            url: Some("http://example.com/test".to_string()),
            interval: Some(42),
            tolerance: Some(7),
        };
        let net = c.proxy_group_to_net(pg_urltest, &proxy_map).unwrap();
        assert_eq!(net.net_type, "url-test");
        assert_eq!(
            net.opt.get("url").and_then(|v| v.as_str()),
            Some("http://example.com/test")
        );
        assert_eq!(net.opt.get("interval").and_then(|v| v.as_u64()), Some(42));
        assert_eq!(net.opt.get("tolerance").and_then(|v| v.as_u64()), Some(7));

        let pg_fallback = ProxyGroup {
            name: "g".to_string(),
            proxy_group_type: "fallback".to_string(),
            proxies: vec!["a".to_string()],
            url: Some("http://example.com/fallback".to_string()),
            interval: Some(24),
            tolerance: None,
        };
        let net = c.proxy_group_to_net(pg_fallback, &proxy_map).unwrap();
        assert_eq!(net.net_type, "fallback");
        assert_eq!(
            net.opt.get("url").and_then(|v| v.as_str()),
            Some("http://example.com/fallback")
        );
        assert_eq!(net.opt.get("interval").and_then(|v| v.as_u64()), Some(24));

        let pg_relay_missing_proxy = ProxyGroup {
            name: "g".to_string(),
            proxy_group_type: "relay".to_string(),
            proxies: vec!["b".to_string()],
            url: None,
            interval: None,
            tolerance: None,
        };
        assert!(c
            .proxy_group_to_net(pg_relay_missing_proxy, &proxy_map)
            .is_err());

        let pg_bad = ProxyGroup {
            name: "g".to_string(),
            proxy_group_type: "bad".to_string(),
            proxies: vec!["a".to_string()],
            url: None,
            interval: None,
            tolerance: None,
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
        assert_eq!(item.len(), 1);
        match item.into_iter().next().unwrap().matcher {
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
        assert_eq!(item.len(), 1);
        assert!(matches!(
            item.into_iter().next().unwrap().matcher,
            Matcher::IpCidr(_)
        ));

        let item = c
            .rule_to_rule(
                "SRC-IP-CIDR,10.0.0.0/8,ProxyA".to_string(),
                &cache,
                &BTreeMap::new(),
                &oom_lock,
            )
            .await
            .unwrap();
        assert_eq!(item.len(), 1);
        assert!(matches!(
            item.into_iter().next().unwrap().matcher,
            Matcher::SrcIpCidr(_)
        ));

        let item = c
            .rule_to_rule(
                "MATCH,ProxyA".to_string(),
                &cache,
                &BTreeMap::new(),
                &oom_lock,
            )
            .await
            .unwrap();
        assert_eq!(item.len(), 1);
        assert!(matches!(
            item.into_iter().next().unwrap().matcher,
            Matcher::Any(_)
        ));

        let item = c
            .rule_to_rule(
                "GEOIP,CN,ProxyA".to_string(),
                &cache,
                &BTreeMap::new(),
                &oom_lock,
            )
            .await
            .unwrap();
        assert_eq!(item.len(), 1);
        assert!(matches!(
            item.into_iter().next().unwrap().matcher,
            Matcher::GeoIp(_)
        ));

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

        assert_eq!(item.len(), 1);
        match item.into_iter().next().unwrap().matcher {
            Matcher::Domain(d) => {
                assert!(matches!(d.method, DomainMatcherMethod::Match));
                assert!(d.domain.len() >= 2);
            }
            _ => panic!("unexpected matcher"),
        }
    }

    #[tokio::test]
    async fn test_rule_to_rule_rule_set_classical_inherits_outer_target() {
        let mut c = base_clash();
        c.name_map.insert("ProxyA".to_string(), "pA".to_string());

        let cache = crate::storage::MemoryCache::new().await.unwrap();
        let oom_lock = Mutex::new(());

        let mut rules = BTreeMap::new();
        let mut tmp = NamedTempFile::new().unwrap();
        std::io::Write::write_all(
            &mut tmp,
            b"payload:\n  - DOMAIN-SUFFIX,example.com\n  - GEOIP,CN\n",
        )
        .unwrap();

        rules.insert(
            "set1".to_string(),
            RuleProvider {
                rule_type: "file".to_string(),
                behavior: "classical".to_string(),
                url: "".to_string(),
                path: tmp.path().to_string_lossy().to_string(),
                interval: 1,
            },
        );

        let items = c
            .rule_to_rule(
                "RULE-SET,set1,ProxyA".to_string(),
                &cache,
                &rules,
                &oom_lock,
            )
            .await
            .unwrap();

        assert_eq!(items.len(), 2);
        assert!(items.iter().all(|item| item.target.represent() == "pA"));
        assert!(matches!(items[0].matcher, Matcher::Domain(_)));
        assert!(matches!(items[1].matcher, Matcher::GeoIp(_)));
    }

    #[tokio::test]
    async fn test_process_flattens_classical_rule_set_rules() {
        let mut clash = base_clash();
        clash.rule_name = Some("rules".to_string());

        let mut tmp = NamedTempFile::new().unwrap();
        std::io::Write::write_all(
            &mut tmp,
            b"payload:\n  - DOMAIN-SUFFIX,example.com\n  - DOMAIN-SUFFIX,example.org\n",
        )
        .unwrap();

        let content = format!(
            r#"proxies:
  - name: ProxyA
    type: direct
proxy-groups: []
rules:
  - RULE-SET,set1,ProxyA
rule-providers:
  set1:
    type: file
    behavior: classical
    path: {}
    url: ""
    interval: 1
"#,
            tmp.path().display()
        );

        let mut config = Config::default();
        let cache = crate::storage::MemoryCache::new().await.unwrap();
        clash.process(&mut config, &content, &cache).await.unwrap();

        let rule_net = config.net.get("rules").unwrap();
        assert_eq!(rule_net.net_type, "rule");

        let rule = rule_net
            .opt
            .get("rule")
            .and_then(|value| value.as_array())
            .unwrap();
        assert_eq!(rule.len(), 1);

        let domains = rule[0]
            .get("domain")
            .and_then(|value| value.as_array())
            .unwrap();
        assert_eq!(domains.len(), 2);
    }
}
