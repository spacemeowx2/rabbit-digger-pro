use std::{net::IpAddr, sync::Arc, time::Duration};

use lru_time_cache::LruCache;
use parking_lot::Mutex;
use trust_dns_proto::{
    op::Message,
    rr::RData,
    serialize::binary::{BinDecodable, BinDecoder},
};

struct Inner {
    records: LruCache<IpAddr, String>,
    cname_map: LruCache<String, String>,
}

#[derive(Clone)]
pub struct ReverseLookup {
    inner: Arc<Mutex<Inner>>,
}

impl ReverseLookup {
    pub fn new() -> ReverseLookup {
        ReverseLookup {
            inner: Arc::new(Mutex::new(Inner {
                records: LruCache::with_expiry_duration_and_capacity(
                    Duration::from_secs(10 * 60),
                    128,
                ),
                cname_map: LruCache::with_expiry_duration_and_capacity(
                    Duration::from_secs(10 * 60),
                    128,
                ),
            })),
        }
    }
    pub fn record_packet(&self, packet: &[u8]) {
        let mut decoder = BinDecoder::new(packet);
        let msg = match Message::read(&mut decoder) {
            Ok(msg) if msg.queries().len() == 1 => msg,
            _ => return,
        };

        // It seems to be ok to assume that only one question is present
        // https://stackoverflow.com/questions/4082081/requesting-a-and-aaaa-records-in-single-dns-query/4083071#4083071
        let domain = msg.queries().first().unwrap().name().to_utf8();
        let domain = domain.trim_end_matches('.');

        let Inner { records, cname_map } = &mut *self.inner.lock();
        for rdata in msg.answers().iter().flat_map(|i| i.data()) {
            match rdata {
                RData::A(addr) => {
                    records.insert(IpAddr::V4((*addr).0), domain.to_string());
                }
                RData::AAAA(addr) => {
                    records.insert(IpAddr::V6((*addr).0), domain.to_string());
                }
                RData::CNAME(cname) => {
                    let cname = cname.to_utf8().trim_end_matches('.').to_string();
                    cname_map.insert(cname, domain.to_string());
                }
                _ => {}
            }
        }
    }
    pub fn reverse_lookup(&self, addr: IpAddr) -> Option<String> {
        let Inner { records, cname_map } = &mut *self.inner.lock();
        match records.get(&addr) {
            Some(mut d) => {
                let mut limit = 16;
                while let Some(r) = cname_map.peek(d) {
                    if r == d || limit == 0 {
                        break;
                    }
                    d = r;
                    limit -= 1;
                }
                Some(d.to_string())
            }
            None => None,
        }
    }
}
