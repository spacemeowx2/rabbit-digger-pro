use std::convert::TryFrom;

use super::config::{DomainMatcher, DomainMatcherMethod as Method};
use super::matcher::{MatchContext, Matcher, MaybeAsync};
use anyhow::Result;

impl TryFrom<String> for Method {
    type Error = anyhow::Error;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Ok(match value.as_ref() {
            "keyword" => Method::Keyword,
            "suffix" => Method::Suffix,
            "match" => Method::Match,
            _ => return Err(anyhow::anyhow!("Unsupported method: {}", value)),
        })
    }
}

impl DomainMatcher {
    fn test(&self, domain: &str) -> bool {
        let mut domains = self.domain.iter();
        match self.method {
            Method::Keyword => domains.any(|d| domain.contains(d)),
            Method::Match => domains.any(|d| {
                if d.starts_with("+.") {
                    d.strip_prefix('+')
                        .map(|i| domain.ends_with(i))
                        .unwrap_or(false)
                        || d.strip_prefix("+.").map(|d| domain == d).unwrap_or(false)
                } else {
                    domain == d
                }
            }),
            Method::Suffix => domains.any(|d| domain.ends_with(d)),
        }
    }
}

impl Matcher for DomainMatcher {
    fn match_rule(&self, match_context: &MatchContext) -> MaybeAsync<bool> {
        match match_context.get_domain() {
            Some((domain, _)) => self.test(domain),
            // if it's not a domain, pass it.
            None => false,
        }
        .into()
    }
}

#[cfg(test)]
mod tests {
    use rd_interface::{Context, IntoAddress};

    use super::*;

    async fn match_addr(address: &str, matcher: &DomainMatcher) -> bool {
        let mut match_context =
            MatchContext::from_context_address(&Context::new(), &address.into_address().unwrap())
                .unwrap();
        matcher.match_rule(&mut match_context).await
    }

    #[tokio::test]
    async fn test_domain_matcher() {
        // test keyword
        let matcher = DomainMatcher {
            domain: vec!["example".to_string()].into(),
            method: Method::Keyword,
        };
        assert!(match_addr("example.com:26666", &matcher).await);
        assert!(!match_addr("exampl.com:26666", &matcher).await);

        // test match
        let matcher = DomainMatcher {
            domain: vec!["example.com".to_string()].into(),
            method: Method::Match,
        };
        assert!(match_addr("example.com:26666", &matcher).await);
        assert!(!match_addr("sub.example.com:26666", &matcher).await);

        // test suffix
        let matcher = DomainMatcher {
            domain: vec!["example.com".to_string()].into(),
            method: Method::Suffix,
        };
        assert!(match_addr("example.com:26666", &matcher).await);
        assert!(match_addr("sub.example.com:26666", &matcher).await);
        assert!(match_addr("prefixexample.com:26666", &matcher).await);
        assert!(!match_addr("example.cn:26666", &matcher).await);

        // test suffix with +
        let matcher = DomainMatcher {
            domain: vec!["+.example.com".to_string()].into(),
            method: Method::Match,
        };
        assert!(match_addr("example.com:26666", &matcher).await);
        assert!(match_addr("sub.example.com:26666", &matcher).await);
        assert!(!match_addr("prefixexample.com:26666", &matcher).await);
        assert!(!match_addr("example.cn:26666", &matcher).await);
    }
}
