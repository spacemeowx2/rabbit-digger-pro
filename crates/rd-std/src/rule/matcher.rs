use futures::{future::BoxFuture, Future, FutureExt};
use rd_interface::{
    context::common_field::{DestDomain, DestSocketAddr, SrcSocketAddr},
    Address, AddressDomain, Result,
};
use std::{
    net::{IpAddr, SocketAddr},
    pin, task,
};

pub(super) enum MaybeAsync<T> {
    Sync {
        value: Option<T>,
    },
    #[allow(dead_code)]
    Async {
        future: BoxFuture<'static, T>,
    },
}

impl<T> From<T> for MaybeAsync<T> {
    fn from(value: T) -> Self {
        MaybeAsync::Sync { value: Some(value) }
    }
}

impl<T: Unpin> Future for MaybeAsync<T> {
    type Output = T;

    fn poll(self: pin::Pin<&mut Self>, cx: &mut task::Context<'_>) -> task::Poll<Self::Output> {
        match self.get_mut() {
            MaybeAsync::Sync { value } => {
                task::Poll::Ready(value.take().expect("Don't poll twice on MaybeAsync"))
            }
            MaybeAsync::Async { future } => future.poll_unpin(cx),
        }
    }
}

pub(super) trait Matcher: Send + Sync {
    fn match_rule(&self, match_context: &MatchContext) -> MaybeAsync<bool>;
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct MatchContext {
    address: Address,
    src_ip_addr: Option<IpAddr>,
    dest_socket_addr: Option<SocketAddr>,
    dest_domain: Option<AddressDomain>,
}

impl MatchContext {
    pub fn from_context_address(
        ctx: &rd_interface::Context,
        addr: &Address,
    ) -> Result<MatchContext> {
        Ok(MatchContext {
            address: addr.to_normalized(),
            src_ip_addr: ctx.get_common::<SrcSocketAddr>()?.map(|v| v.0.ip()),
            dest_socket_addr: ctx.get_common::<DestSocketAddr>()?.map(|v| v.0),
            dest_domain: ctx.get_common::<DestDomain>()?.map(|v| v.0),
        })
    }
    pub fn address(&self) -> &Address {
        &self.address
    }
    pub fn src_ip_addr(&self) -> Option<&IpAddr> {
        self.src_ip_addr.as_ref()
    }
    pub fn dest_socket_addr(&self) -> Option<&SocketAddr> {
        self.dest_socket_addr.as_ref()
    }
    pub fn dest_domain(&self) -> Option<&AddressDomain> {
        self.dest_domain.as_ref()
    }
    pub fn get_domain(&self) -> Option<(&String, &u16)> {
        match self.address() {
            Address::Domain(d, p) => return Some((d, p)),
            Address::SocketAddr(_) => {}
        };
        match self.dest_domain() {
            Some(AddressDomain { domain, port }) => return Some((domain, port)),
            None => {}
        };

        None
    }
    pub fn get_socket_addr(&self) -> Option<SocketAddr> {
        match self.address() {
            Address::SocketAddr(addr) => return Some(*addr),
            Address::Domain(_, _) => match self.address.to_socket_addr() {
                Ok(addr) => return Some(addr),
                Err(_) => {}
            },
        };
        match self.dest_socket_addr() {
            Some(addr) => return Some(*addr),
            None => {}
        };

        None
    }
}
