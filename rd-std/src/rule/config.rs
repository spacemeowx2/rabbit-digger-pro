use std::{borrow::Cow, fmt, str::FromStr};

use super::matcher::{self, MatchContext};
use rd_interface::{
    config::{CompactVecString, NetRef, SingleOrVec},
    impl_empty_config,
    prelude::*,
    schemars::JsonSchema,
};
use serde_with::{DeserializeFromStr, SerializeDisplay};
use smoltcp::wire;

#[rd_config]
#[derive(Debug, Clone)]
#[serde(rename_all = "lowercase")]
pub enum DomainMatcherMethod {
    Keyword,
    Suffix,
    Match,
}

#[rd_config]
#[derive(Debug)]
pub struct DomainMatcher {
    pub method: DomainMatcherMethod,
    pub domain: CompactVecString,
}

#[derive(Debug, Clone, SerializeDisplay, DeserializeFromStr)]
pub struct IpCidr(pub wire::IpCidr);

impl fmt::Display for IpCidr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl FromStr for IpCidr {
    type Err = rd_interface::Error;

    /// Parse a string representation of an IP CIDR.
    fn from_str(s: &str) -> rd_interface::Result<IpCidr> {
        if let Ok(cidr) = wire::Ipv4Cidr::from_str(s) {
            return Ok(IpCidr(wire::IpCidr::Ipv4(cidr)));
        }

        if let Ok(cidr) = wire::Ipv6Cidr::from_str(s) {
            return Ok(IpCidr(wire::IpCidr::Ipv6(cidr)));
        }

        Err(rd_interface::Error::Other(
            format!("Failed to parse ip_cidr: {s}").into(),
        ))
    }
}

impl_empty_config! { IpCidr }

#[rd_config]
#[derive(Debug, Clone)]
pub struct IpCidrMatcher {
    pub ipcidr: SingleOrVec<IpCidr>,
}

#[rd_config]
#[derive(Debug, Clone)]
pub struct SrcIpCidrMatcher {
    pub ipcidr: SingleOrVec<IpCidr>,
}

#[rd_config]
#[derive(Debug, Clone)]
pub struct GeoIpMatcher {
    pub country: String,
}

impl JsonSchema for IpCidr {
    fn schema_name() -> Cow<'static, str> {
        Cow::Borrowed("IpCidr")
    }

    fn json_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
        let mut obj = serde_json::Map::new();
        obj.insert(
            "type".to_string(),
            serde_json::Value::String("string".to_string()),
        );
        schemars::Schema::from(obj)
    }
}

#[rd_config]
#[derive(Debug, Clone)]
pub struct AnyMatcher {}

#[rd_config]
#[derive(Debug)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Matcher {
    Domain(DomainMatcher),
    IpCidr(IpCidrMatcher),
    #[serde(rename = "src_ipcidr")]
    SrcIpCidr(SrcIpCidrMatcher),
    GeoIp(GeoIpMatcher),
    Any(AnyMatcher),
}

impl Matcher {
    pub fn merge(&mut self, other: &Matcher) -> bool {
        match (self, other) {
            (Matcher::Domain(ref mut self_domain), Matcher::Domain(ref other_domain)) => {
                self_domain.domain.extend(&other_domain.domain);
                true
            }
            (Matcher::IpCidr(ref mut self_ipcidr), Matcher::IpCidr(ref other_ipcidr)) => {
                self_ipcidr
                    .ipcidr
                    .extend(other_ipcidr.ipcidr.iter().cloned());
                true
            }
            (
                Matcher::SrcIpCidr(ref mut self_srcipcidr),
                Matcher::SrcIpCidr(ref other_srcipcidr),
            ) => {
                self_srcipcidr
                    .ipcidr
                    .extend(other_srcipcidr.ipcidr.iter().cloned());
                true
            }
            (Matcher::Any(_), Matcher::Any(_)) => true,
            (Matcher::GeoIp(_), Matcher::GeoIp(_)) => false,
            _ => false,
        }
    }
}

#[rd_config]
#[derive(Debug)]
pub struct RuleItem {
    pub target: NetRef,
    #[serde(flatten)]
    pub matcher: Matcher,
}

impl RuleItem {
    pub fn merge(&mut self, other: &RuleItem) -> bool {
        if self.target.represent() == other.target.represent() {
            self.matcher.merge(&other.matcher)
        } else {
            false
        }
    }
}

#[rd_config]
#[derive(Debug)]
pub struct RuleNetConfig {
    #[serde(default = "default_lru_cache_size")]
    pub lru_cache_size: usize,
    #[serde(skip_serializing_if = "rd_interface::config::detailed_field")]
    pub rule: Vec<RuleItem>,
}

fn default_lru_cache_size() -> usize {
    32
}

impl matcher::Matcher for Matcher {
    fn match_rule(&self, match_context: &MatchContext) -> matcher::MaybeAsync<bool> {
        match self {
            Matcher::Domain(i) => i.match_rule(match_context),
            Matcher::IpCidr(i) => i.match_rule(match_context),
            Matcher::SrcIpCidr(i) => i.match_rule(match_context),
            Matcher::GeoIp(i) => i.match_rule(match_context),
            Matcher::Any(i) => i.match_rule(match_context),
        }
    }
}

impl Matcher {
    pub fn shrink_to_fit(&mut self) {
        match self {
            Matcher::Domain(i) => i.shrink_to_fit(),
            Matcher::IpCidr(i) => i.shrink_to_fit(),
            Matcher::SrcIpCidr(i) => i.shrink_to_fit(),
            _ => {}
        }
    }
}

impl DomainMatcher {
    pub fn shrink_to_fit(&mut self) {
        self.domain.shrink_to_fit();
    }
}

impl IpCidrMatcher {
    pub fn shrink_to_fit(&mut self) {
        self.ipcidr.shrink_to_fit()
    }
}

impl SrcIpCidrMatcher {
    pub fn shrink_to_fit(&mut self) {
        self.ipcidr.shrink_to_fit()
    }
}
