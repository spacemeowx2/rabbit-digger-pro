use crate::{rule::matcher::MatchContext, util::UdpConnector};

use super::config;
use super::matcher::Matcher;

use lru_time_cache::LruCache;
use parking_lot::Mutex;
use rd_interface::{
    async_trait, Address, Arc, Context, INet, IntoDyn, Net, Result, TcpStream, UdpSocket,
};
use tracing::instrument;

pub struct RuleItem {
    #[allow(dead_code)]
    pub target_name: String,
    pub target: Net,
    matcher: config::Matcher,
}

#[derive(Clone)]
pub struct Rule {
    rule: Arc<Vec<RuleItem>>,
    cache: Arc<Mutex<LruCache<MatchContext, usize>>>,
}

impl Rule {
    fn new(config: config::RuleNetConfig) -> Result<Rule> {
        if config
            .rule
            .iter()
            .any(|i| matches!(i.matcher, config::Matcher::GeoIp(_)))
        {
            // if used geoip, init reader first.
            super::geoip::get_reader();
        }
        let mut rule = config
            .rule
            .into_iter()
            .map(
                |config::RuleItem {
                     target,
                     mut matcher,
                 }| {
                    matcher.shrink_to_fit();
                    Ok(RuleItem {
                        matcher,
                        target: target.value_cloned(),
                        target_name: target.represent().to_string(),
                    })
                },
            )
            .collect::<Result<Vec<_>>>()?;

        rule.shrink_to_fit();

        let rule = Arc::new(rule);
        let cache = Arc::new(Mutex::new(LruCache::with_capacity(config.lru_cache_size)));

        Ok(Rule { rule, cache })
    }
    #[instrument(skip(self), err)]
    pub async fn get_rule(&self, ctx: &Context, target: &Address) -> Result<&RuleItem> {
        let match_context = MatchContext::from_context_address(ctx, target)?;

        // hit cache
        if let Some(i) = self.cache.lock().get(&match_context).copied() {
            let rule = &self.rule[i];
            tracing::trace!(matcher = ?rule.matcher, hit_cache = true, "matched rule");
            return Ok(rule);
        }

        for (i, rule) in self.rule.iter().enumerate() {
            if rule.matcher.match_rule(&match_context).await {
                self.cache.lock().insert(match_context, i);
                tracing::trace!(matcher = ?rule.matcher, hit_cache = false, "matched rule");
                return Ok(rule);
            }
        }

        tracing::trace!("Not matched");
        Err(rd_interface::Error::NotMatched)
    }
}

pub struct RuleNet {
    rule: Rule,
}

impl RuleNet {
    pub fn new(config: config::RuleNetConfig) -> Result<RuleNet> {
        Ok(RuleNet {
            rule: Rule::new(config)?,
        })
    }
}

#[async_trait]
impl rd_interface::TcpConnect for RuleNet {
    async fn tcp_connect(&self, ctx: &mut Context, addr: &Address) -> Result<TcpStream> {
        self.rule
            .get_rule(ctx, addr)
            .await?
            .target
            .tcp_connect(ctx, addr)
            .await
    }
}

#[async_trait]
impl rd_interface::UdpBind for RuleNet {
    async fn udp_bind(&self, ctx: &mut Context, bind_addr: &Address) -> Result<UdpSocket> {
        let rule = self.rule.clone();
        let mut ctx = ctx.clone();
        let bind_addr = bind_addr.clone();
        let udp = UdpConnector::new(Box::new(move |buf: &[u8], target_addr: &Address| {
            let buf = buf.to_vec();
            let target_addr = target_addr.clone();
            Box::pin(async move {
                let rule_item = rule.get_rule(&ctx, &target_addr).await?;
                let mut udp = rule_item.target.udp_bind(&mut ctx, &bind_addr).await?;
                udp.send_to(&buf, &target_addr).await?;
                Ok(udp)
            })
        }));
        Ok(udp.into_dyn())
    }
}

impl INet for RuleNet {
    fn provide_tcp_connect(&self) -> Option<&dyn rd_interface::TcpConnect> {
        Some(self)
    }

    fn provide_udp_bind(&self) -> Option<&dyn rd_interface::UdpBind> {
        Some(self)
    }
}

#[cfg(test)]
mod tests {
    use rd_interface::{config::NetRef, IntoAddress};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    use crate::{
        tests::{
            assert_echo, assert_echo_udp, assert_net_provider, spawn_echo_server,
            spawn_echo_server_udp, ProviderCapability, TestNet,
        },
        util::NotImplementedNet,
    };

    use super::*;

    #[test]
    fn test_provider() {
        let rule_config = config::RuleNetConfig {
            rule: vec![],
            lru_cache_size: 10,
        };
        let rule_net = RuleNet::new(rule_config).unwrap().into_dyn();

        assert_net_provider(
            &rule_net,
            ProviderCapability {
                tcp_connect: true,
                udp_bind: true,
                ..Default::default()
            },
        );
    }

    #[tokio::test]
    async fn test_rule_net() {
        let net = TestNet::new().into_dyn();
        let noop = NotImplementedNet.into_dyn();

        spawn_echo_server_udp(&net, "127.0.0.1:12345").await;

        let rule_config = config::RuleNetConfig {
            rule: vec![
                config::RuleItem {
                    matcher: config::Matcher::GeoIp(config::GeoIpMatcher {
                        country: "CN".to_string(),
                    }),
                    target: NetRef::new_with_value("noop".into(), noop.clone()),
                },
                config::RuleItem {
                    matcher: config::Matcher::Any(config::AnyMatcher {}),
                    target: NetRef::new_with_value("test".into(), net.clone()),
                },
            ],
            lru_cache_size: 10,
        };
        let rule_net = RuleNet::new(rule_config).unwrap().into_dyn();

        assert_echo_udp(&rule_net, "127.0.0.1:12345").await;
        // the second time should hit cache
        assert_echo_udp(&rule_net, "127.0.0.1:12345").await;

        let err = rule_net
            .tcp_connect(
                &mut Context::new(),
                &"114.114.114.114:53".into_address().unwrap(),
            )
            .await;
        assert!(matches!(err, Err(rd_interface::Error::NotImplemented)));
    }

    #[tokio::test]
    async fn test_rule_net_not_match() {
        let rule_config = config::RuleNetConfig {
            rule: vec![],
            lru_cache_size: 10,
        };
        let rule_net = RuleNet::new(rule_config).unwrap().into_dyn();

        let err = rule_net
            .tcp_connect(
                &mut Context::new(),
                &"114.114.114.114:53".into_address().unwrap(),
            )
            .await;
        assert!(matches!(err, Err(rd_interface::Error::NotMatched)));
    }

    #[tokio::test]
    async fn test_rule_types() {
        let net = TestNet::new().into_dyn();

        spawn_echo_server(&net, "127.0.0.1:12345").await;

        let rule_net = RuleNet::new(config::RuleNetConfig {
            rule: vec![config::RuleItem {
                matcher: config::Matcher::IpCidr(config::IpCidrMatcher {
                    ipcidr: vec!["127.0.0.1/32".parse().unwrap()].into(),
                }),
                target: NetRef::new_with_value("net".into(), net.clone()),
            }],
            lru_cache_size: 10,
        })
        .unwrap()
        .into_dyn();

        assert_echo(&rule_net, "127.0.0.1:12345").await;

        let rule_net = RuleNet::new(config::RuleNetConfig {
            rule: vec![config::RuleItem {
                matcher: config::Matcher::SrcIpCidr(config::SrcIpCidrMatcher {
                    ipcidr: vec!["127.0.0.1/32".parse().unwrap()].into(),
                }),
                target: NetRef::new_with_value("net".into(), net.clone()),
            }],
            lru_cache_size: 10,
        })
        .unwrap()
        .into_dyn();

        const BUF: &[u8] = b"asdfasdfasdfasj12312313123";
        let mut tcp = rule_net
            .tcp_connect(
                &mut Context::from_socketaddr("127.0.0.1:1".parse().unwrap()),
                &"127.0.0.1:12345".into_address().unwrap(),
            )
            .await
            .unwrap();
        tcp.write_all(BUF).await.unwrap();

        let mut buf = [0u8; BUF.len()];
        tcp.read_exact(&mut buf).await.unwrap();

        assert_eq!(buf, BUF);

        let rule_net = RuleNet::new(config::RuleNetConfig {
            rule: vec![config::RuleItem {
                matcher: config::Matcher::Domain(config::DomainMatcher {
                    method: config::DomainMatcherMethod::Match,
                    domain: vec!["localhost".to_string()].into(),
                }),
                target: NetRef::new_with_value("net".into(), net.clone()),
            }],
            lru_cache_size: 10,
        })
        .unwrap()
        .into_dyn();

        assert_echo(&rule_net, "localhost:12345").await;
    }

    #[tokio::test]
    async fn test_normalize() {
        let net = TestNet::new().into_dyn();

        spawn_echo_server(&net, "127.0.0.1:12345").await;

        let rule_net = RuleNet::new(config::RuleNetConfig {
            rule: vec![config::RuleItem {
                matcher: config::Matcher::IpCidr(config::IpCidrMatcher {
                    ipcidr: vec!["127.0.0.1/32".parse().unwrap()].into(),
                }),
                target: NetRef::new_with_value("net".into(), net.clone()),
            }],
            lru_cache_size: 10,
        })
        .unwrap()
        .into_dyn();

        let addr = Address::Domain("127.0.0.1".to_string(), 12345);
        assert_echo(&rule_net, addr).await;
    }
}
