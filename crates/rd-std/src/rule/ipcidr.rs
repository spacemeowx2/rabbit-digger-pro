use super::config::{IpCidrMatcher, SrcIpCidrMatcher};
use super::matcher::{MatchContext, Matcher, MaybeAsync};
use smoltcp::wire::IpAddress;

impl IpCidrMatcher {
    fn test(&self, address: impl Into<IpAddress>) -> bool {
        let address: IpAddress = address.into();
        self.ipcidr.iter().any(|i| i.0.contains_addr(&address))
    }
}

impl Matcher for IpCidrMatcher {
    fn match_rule(&self, match_context: &MatchContext) -> MaybeAsync<bool> {
        match match_context.get_socket_addr() {
            Some(addr) => self.test(addr.ip()),
            _ => false,
        }
        .into()
    }
}

impl SrcIpCidrMatcher {
    fn test(&self, address: impl Into<IpAddress>) -> bool {
        let address: IpAddress = address.into();
        self.ipcidr.iter().any(|i| i.0.contains_addr(&address))
    }
}

impl Matcher for SrcIpCidrMatcher {
    fn match_rule(&self, match_context: &MatchContext) -> MaybeAsync<bool> {
        match match_context.src_ip_addr() {
            Some(addr) => self.test(*addr),
            None => false,
        }
        .into()
    }
}

#[cfg(test)]
mod tests {
    #[tokio::test]
    async fn test_ipcidr() {
        use super::*;
        use crate::rule::config;
        use rd_interface::{Context, IntoAddress};
        use smoltcp::wire::{IpAddress, IpCidr};

        let matcher = IpCidrMatcher {
            ipcidr: vec![config::IpCidr(IpCidr::new(
                IpAddress::v4(192, 168, 1, 1),
                24,
            ))]
            .into(),
        };

        assert_eq!(
            matcher
                .match_rule(
                    &MatchContext::from_context_address(
                        &Context::new(),
                        &"192.168.1.2:26666".into_address().unwrap()
                    )
                    .unwrap()
                )
                .await,
            true
        );

        assert_eq!(
            matcher
                .match_rule(
                    &MatchContext::from_context_address(
                        &Context::new(),
                        &"192.168.2.2:1234".into_address().unwrap()
                    )
                    .unwrap()
                )
                .await,
            false
        );

        assert_eq!(
            matcher
                .match_rule(
                    &MatchContext::from_context_address(
                        &Context::new(),
                        &"192.168.1.1:1234".into_address().unwrap()
                    )
                    .unwrap()
                )
                .await,
            true
        );
    }
}
