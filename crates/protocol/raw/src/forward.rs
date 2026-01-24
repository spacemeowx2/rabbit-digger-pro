use std::{
    io,
    net::{Ipv4Addr, SocketAddr, SocketAddrV4},
};

use rd_interface::{Arc, Context, IntoAddress, Net, Result, TcpStream};
use rd_std::{util::forward_udp, ContextExt};
use tokio::select;
use tokio_smoltcp::{
    smoltcp::wire::{IpCidr, IpProtocol, IpVersion},
    Net as SmoltcpNet, RawSocket, TcpListener,
};

use crate::gateway::MapTable;

mod source;

struct Forward {
    net: Net,
    map: MapTable,
    ip_cidr: IpCidr,
}

pub async fn forward_net(
    net: Net,
    smoltcp_net: Arc<SmoltcpNet>,
    map: MapTable,
    ip_cidr: IpCidr,
) -> io::Result<()> {
    let tcp_listener = smoltcp_net
        .tcp_bind(SocketAddrV4::new(Ipv4Addr::new(0, 0, 0, 0), 1).into())
        .await?;
    let raw_socket = smoltcp_net
        .raw_socket(IpVersion::Ipv4, IpProtocol::Udp)
        .await?;

    let forward = Forward { net, map, ip_cidr };

    let tcp_task = forward.serve_tcp(tcp_listener);
    let udp_task = forward.serve_udp(raw_socket);

    select! {
        r = tcp_task => r?,
        r = udp_task => r?,
    };

    Ok(())
}

impl Forward {
    async fn serve_tcp(&self, mut listener: TcpListener) -> Result<()> {
        loop {
            let (tcp, addr) = listener.accept().await?;
            let orig_addr = self.map.get(&match addr {
                SocketAddr::V4(v4) => v4,
                _ => continue,
            });
            if let Some(orig_addr) = orig_addr {
                let net = self.net.clone();
                tokio::spawn(async move {
                    let ctx = &mut Context::from_socketaddr(addr);
                    let target = net
                        .tcp_connect(ctx, &SocketAddr::from(orig_addr).into_address()?)
                        .await?;
                    ctx.connect_tcp(TcpStream::from(tcp), target).await?;
                    Ok(()) as Result<()>
                });
            }
        }
    }
    async fn serve_udp(&self, raw: RawSocket) -> Result<()> {
        let source = source::Source::new(raw, self.ip_cidr);

        forward_udp::forward_udp(source, self.net.clone(), None).await?;

        Ok(())
    }
}
