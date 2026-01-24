use std::{
    net::SocketAddr,
    task::{self, Poll},
};

use super::service::ReverseLookup;
use futures::ready;
use rd_interface::{
    async_trait, context::common_field::DestDomain, Address, AddressDomain, Context, INet,
    IUdpSocket, IntoDyn, Net, Result, UdpSocket,
};

/// This net is used for reverse lookup.
///
/// When a UDP packet recv from port 53, the DNS response will be recorded in this net.
/// And the DNS response will be sent to the client.
/// The tcp_connect to recorded IP will be recovered to domain name.
/// If the domain name is in the cache, this net will add "DestDomain" to the context.
pub struct DNSSnifferNet {
    net: Net,
    rl: ReverseLookup,
}

impl DNSSnifferNet {
    pub fn new(net: Net) -> Self {
        Self {
            net,
            rl: ReverseLookup::new(),
        }
    }
    fn reverse_lookup(&self, ctx: &mut Context, addr: &Address) -> Address {
        match addr {
            Address::SocketAddr(sa) => self
                .rl
                .reverse_lookup(sa.ip())
                .map(|name| {
                    let domain = Address::Domain(name.clone(), sa.port());
                    ctx.insert_common(DestDomain(AddressDomain {
                        domain: name,
                        port: sa.port(),
                    }))
                    .expect("Failed to insert domain");
                    tracing::trace!(?domain, "recovered domain");
                    domain
                })
                .unwrap_or_else(|| addr.clone()),
            d => d.clone(),
        }
    }
}

#[async_trait]
impl rd_interface::TcpConnect for DNSSnifferNet {
    async fn tcp_connect(
        &self,
        ctx: &mut Context,
        addr: &Address,
    ) -> Result<rd_interface::TcpStream> {
        let addr = &self.reverse_lookup(ctx, addr);
        self.net.tcp_connect(ctx, addr).await
    }
}

#[async_trait]
impl rd_interface::UdpBind for DNSSnifferNet {
    async fn udp_bind(&self, ctx: &mut Context, addr: &Address) -> Result<UdpSocket> {
        let udp = self.net.udp_bind(ctx, addr).await?;
        Ok(MitmUdp(udp, self.rl.clone()).into_dyn())
    }
}

impl INet for DNSSnifferNet {
    fn provide_tcp_connect(&self) -> Option<&dyn rd_interface::TcpConnect> {
        Some(self)
    }

    fn provide_tcp_bind(&self) -> Option<&dyn rd_interface::TcpBind> {
        self.net.provide_tcp_bind()
    }

    fn provide_udp_bind(&self) -> Option<&dyn rd_interface::UdpBind> {
        Some(self)
    }

    fn provide_lookup_host(&self) -> Option<&dyn rd_interface::LookupHost> {
        self.net.provide_lookup_host()
    }
}

struct MitmUdp(UdpSocket, ReverseLookup);

#[async_trait]
impl IUdpSocket for MitmUdp {
    async fn local_addr(&self) -> Result<SocketAddr> {
        self.0.local_addr().await
    }

    fn poll_recv_from(
        &mut self,
        cx: &mut task::Context<'_>,
        buf: &mut rd_interface::ReadBuf,
    ) -> Poll<std::io::Result<SocketAddr>> {
        let this = &mut *self;
        let from_addr = ready!(this.0.poll_recv_from(cx, buf))?;

        if from_addr.port() == 53 {
            self.1.record_packet(buf.filled());
        }

        Poll::Ready(Ok(from_addr))
    }

    fn poll_send_to(
        &mut self,
        cx: &mut task::Context<'_>,
        buf: &[u8],
        target: &Address,
    ) -> Poll<std::io::Result<usize>> {
        self.0.poll_send_to(cx, buf, target)
    }
}

#[cfg(test)]
mod tests {
    use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

    use rd_interface::{IntoAddress, ReadBuf};

    use crate::tests::{assert_net_provider, ProviderCapability, TestNet};

    use super::*;

    #[test]
    fn test_provider() {
        let test_net = TestNet::new().into_dyn();
        let net = DNSSnifferNet::new(test_net).into_dyn();

        assert_net_provider(
            &net,
            ProviderCapability {
                tcp_connect: true,
                tcp_bind: true,
                udp_bind: true,
                lookup_host: true,
            },
        );
    }

    #[tokio::test]
    async fn test_dns_sniffer() {
        let test_net = TestNet::new().into_dyn();
        let net = DNSSnifferNet::new(test_net.clone());

        let mut ctx = Context::new();
        let mut dns_server = net
            .provide_udp_bind()
            .unwrap()
            .udp_bind(&mut ctx, &"127.0.0.1:53".into_address().unwrap())
            .await
            .unwrap();
        let mut client = net
            .provide_udp_bind()
            .unwrap()
            .udp_bind(&mut ctx, &"127.0.0.1:0".into_address().unwrap())
            .await
            .unwrap();

        // dns request to baidu.com
        client
            .send_to(
                &[
                    0x00, 0x02, 0x01, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x05,
                    0x62, 0x61, 0x69, 0x64, 0x75, 0x03, 0x63, 0x6F, 0x6D, 0x00, 0x00, 0x01, 0x00,
                    0x01,
                ],
                &SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 53).into(),
            )
            .await
            .unwrap();
        let buf = &mut vec![0; 1024];
        let addr = dns_server.recv_from(&mut ReadBuf::new(buf)).await.unwrap();

        assert_eq!(addr, client.local_addr().await.unwrap());

        // dns response to baidu.com. 220.181.38.148, 220.181.38.251
        dns_server
            .send_to(
                &[
                    0x00, 0x02, 0x81, 0x80, 0x00, 0x01, 0x00, 0x02, 0x00, 0x00, 0x00, 0x00, 0x05,
                    0x62, 0x61, 0x69, 0x64, 0x75, 0x03, 0x63, 0x6F, 0x6D, 0x00, 0x00, 0x01, 0x00,
                    0x01, 0xC0, 0x0C, 0x00, 0x01, 0x00, 0x01, 0x00, 0x00, 0x01, 0xFA, 0x00, 0x04,
                    0xDC, 0xB5, 0x26, 0x94, 0xC0, 0x0C, 0x00, 0x01, 0x00, 0x01, 0x00, 0x00, 0x01,
                    0xFA, 0x00, 0x04, 0xDC, 0xB5, 0x26, 0xFB,
                ],
                &addr.into(),
            )
            .await
            .unwrap();
        let _ = client.recv_from(&mut ReadBuf::new(buf)).await.unwrap();

        assert_eq!(
            net.rl
                .reverse_lookup(Ipv4Addr::new(220, 181, 38, 148).into()),
            Some("baidu.com".to_string()),
        );

        assert_eq!(
            net.reverse_lookup(
                &mut Context::new(),
                &"220.181.38.251:443".into_address().unwrap()
            ),
            "baidu.com:443".into_address().unwrap()
        )
    }

    #[tokio::test]
    async fn test_dns_sniffer2() {
        let test_net = TestNet::new().into_dyn();
        let net = DNSSnifferNet::new(test_net.clone());

        let mut ctx = Context::new();
        let mut dns_server = net
            .provide_udp_bind()
            .unwrap()
            .udp_bind(&mut ctx, &"127.0.0.1:53".into_address().unwrap())
            .await
            .unwrap();
        let mut client = net
            .provide_udp_bind()
            .unwrap()
            .udp_bind(&mut ctx, &"127.0.0.1:0".into_address().unwrap())
            .await
            .unwrap();

        // dns request to simple-service-c45xrrmhuc5su.shellweplayaga.me
        client
            .send_to(
                &[
                    0x7E, 0xB9, 0x01, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x1C,
                    0x73, 0x69, 0x6D, 0x70, 0x6C, 0x65, 0x2D, 0x73, 0x65, 0x72, 0x76, 0x69, 0x63,
                    0x65, 0x2D, 0x63, 0x34, 0x35, 0x78, 0x72, 0x72, 0x6D, 0x68, 0x75, 0x63, 0x35,
                    0x73, 0x75, 0x0E, 0x73, 0x68, 0x65, 0x6C, 0x6C, 0x77, 0x65, 0x70, 0x6C, 0x61,
                    0x79, 0x61, 0x67, 0x61, 0x02, 0x6D, 0x65, 0x00, 0x00, 0x01, 0x00, 0x01,
                ],
                &SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 53).into(),
            )
            .await
            .unwrap();
        let buf = &mut vec![0; 1024];
        let addr = dns_server.recv_from(&mut ReadBuf::new(buf)).await.unwrap();

        assert_eq!(addr, client.local_addr().await.unwrap());

        // dns response to simple-service-c45xrrmhuc5su.shellweplayaga.me. 20.187.122.164
        dns_server
            .send_to(
                &[
                    0x7E, 0xB9, 0x81, 0x80, 0x00, 0x01, 0x00, 0x03, 0x00, 0x00, 0x00, 0x00, 0x1C,
                    0x73, 0x69, 0x6D, 0x70, 0x6C, 0x65, 0x2D, 0x73, 0x65, 0x72, 0x76, 0x69, 0x63,
                    0x65, 0x2D, 0x63, 0x34, 0x35, 0x78, 0x72, 0x72, 0x6D, 0x68, 0x75, 0x63, 0x35,
                    0x73, 0x75, 0x0E, 0x73, 0x68, 0x65, 0x6C, 0x6C, 0x77, 0x65, 0x70, 0x6C, 0x61,
                    0x79, 0x61, 0x67, 0x61, 0x02, 0x6D, 0x65, 0x00, 0x00, 0x01, 0x00, 0x01, 0xC0,
                    0x0C, 0x00, 0x05, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x00, 0x31, 0x1C, 0x73,
                    0x69, 0x6D, 0x70, 0x6C, 0x65, 0x2D, 0x73, 0x65, 0x72, 0x76, 0x69, 0x63, 0x65,
                    0x2D, 0x37, 0x32, 0x6F, 0x72, 0x6B, 0x6E, 0x6D, 0x6C, 0x65, 0x74, 0x71, 0x64,
                    0x67, 0x0E, 0x74, 0x72, 0x61, 0x66, 0x66, 0x69, 0x63, 0x6D, 0x61, 0x6E, 0x61,
                    0x67, 0x65, 0x72, 0x03, 0x6E, 0x65, 0x74, 0x00, 0xC0, 0x4C, 0x00, 0x05, 0x00,
                    0x01, 0x00, 0x00, 0x00, 0x01, 0x00, 0x43, 0x25, 0x73, 0x69, 0x6D, 0x70, 0x6C,
                    0x65, 0x2D, 0x73, 0x65, 0x72, 0x76, 0x69, 0x63, 0x65, 0x2D, 0x65, 0x61, 0x73,
                    0x74, 0x61, 0x73, 0x69, 0x61, 0x2D, 0x6C, 0x7A, 0x70, 0x7A, 0x70, 0x6D, 0x34,
                    0x6E, 0x78, 0x67, 0x77, 0x62, 0x36, 0x08, 0x65, 0x61, 0x73, 0x74, 0x61, 0x73,
                    0x69, 0x61, 0x08, 0x63, 0x6C, 0x6F, 0x75, 0x64, 0x61, 0x70, 0x70, 0x05, 0x61,
                    0x7A, 0x75, 0x72, 0x65, 0x03, 0x63, 0x6F, 0x6D, 0x00, 0xC0, 0x89, 0x00, 0x01,
                    0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x00, 0x04, 0x14, 0xBB, 0x7A, 0xA4,
                ],
                &addr.into(),
            )
            .await
            .unwrap();
        let _ = client.recv_from(&mut ReadBuf::new(buf)).await.unwrap();

        assert_eq!(
            net.rl
                .reverse_lookup(Ipv4Addr::new(20, 187, 122, 164).into()),
            Some("simple-service-c45xrrmhuc5su.shellweplayaga.me".to_string()),
        );

        assert_eq!(
            net.reverse_lookup(
                &mut Context::new(),
                &"20.187.122.164:31337".into_address().unwrap()
            ),
            "simple-service-c45xrrmhuc5su.shellweplayaga.me:31337"
                .into_address()
                .unwrap()
        )
    }

    #[tokio::test]
    async fn test_dns_sniffer_v6() {
        let test_net = TestNet::new().into_dyn();
        let net = DNSSnifferNet::new(test_net.clone());

        let mut ctx = Context::new();
        let mut dns_server = net
            .provide_udp_bind()
            .unwrap()
            .udp_bind(&mut ctx, &"127.0.0.1:53".into_address().unwrap())
            .await
            .unwrap();
        let mut client = net
            .provide_udp_bind()
            .unwrap()
            .udp_bind(&mut ctx, &"127.0.0.1:0".into_address().unwrap())
            .await
            .unwrap();

        // dns request to www.google.com
        client
            .send_to(
                &[
                    0xb2, 0xbe, 0x01, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x03,
                    0x77, 0x77, 0x77, 0x06, 0x67, 0x6f, 0x6f, 0x67, 0x6c, 0x65, 0x03, 0x63, 0x6f,
                    0x6d, 0x00, 0x00, 0x1c, 0x00, 0x01,
                ],
                &SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 53).into(),
            )
            .await
            .unwrap();
        let buf = &mut vec![0; 1024];
        let addr = dns_server.recv_from(&mut ReadBuf::new(buf)).await.unwrap();

        assert_eq!(addr, client.local_addr().await.unwrap());

        // dns response to baidu.com. 220.181.38.148, 220.181.38.251
        dns_server
            .send_to(
                &[
                    0xb2, 0xbe, 0x81, 0x80, 0x00, 0x01, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x03,
                    0x77, 0x77, 0x77, 0x06, 0x67, 0x6f, 0x6f, 0x67, 0x6c, 0x65, 0x03, 0x63, 0x6f,
                    0x6d, 0x00, 0x00, 0x1c, 0x00, 0x01, 0xc0, 0x0c, 0x00, 0x1c, 0x00, 0x01, 0x00,
                    0x00, 0x00, 0x22, 0x00, 0x10, 0x20, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
                    0x00, 0x00, 0x00, 0x00, 0x1f, 0x0d, 0x56, 0x15,
                ],
                &addr.into(),
            )
            .await
            .unwrap();
        let _ = client.recv_from(&mut ReadBuf::new(buf)).await.unwrap();

        assert_eq!(
            net.rl
                .reverse_lookup(Ipv6Addr::new(0x2001, 0, 0, 0, 0, 0, 0x1f0d, 0x5615).into()),
            Some("www.google.com".to_string()),
        );

        assert_eq!(
            net.reverse_lookup(
                &mut Context::new(),
                &"[2001::1f0d:5615]:443".into_address().unwrap()
            ),
            "www.google.com:443".into_address().unwrap()
        )
    }
}
