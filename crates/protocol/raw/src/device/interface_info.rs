#![allow(dead_code)]

use tokio_smoltcp::smoltcp::wire::EthernetAddress;

#[derive(Debug)]
pub struct InterfaceInfo {
    pub ethernet_address: EthernetAddress,
    pub name: String,
    pub description: Option<String>,
    pub friendly_name: Option<String>,
}

impl Default for InterfaceInfo {
    fn default() -> Self {
        InterfaceInfo {
            ethernet_address: EthernetAddress::BROADCAST,
            name: "".to_string(),
            description: None,
            friendly_name: None,
        }
    }
}

#[cfg(unix)]
pub use unix::get_interface_info;
#[cfg(windows)]
pub use windows::get_interface_info;

#[cfg(unix)]
mod unix {
    use super::InterfaceInfo;
    use std::io::{self, Error, ErrorKind};
    use tokio_smoltcp::smoltcp::wire::EthernetAddress;

    pub fn get_interface_info(name: &str) -> io::Result<InterfaceInfo> {
        use nix::ifaddrs::getifaddrs;
        let addrs = getifaddrs().map_err(|e| Error::new(ErrorKind::Other, e))?;
        for ifaddr in addrs {
            if ifaddr.interface_name != name {
                continue;
            }
            if let Some(link) = ifaddr
                .address
                .and_then(|a| a.as_link_addr().and_then(|a| a.addr()))
            {
                return Ok(InterfaceInfo {
                    ethernet_address: EthernetAddress(link),
                    name: name.into(),
                    description: None,
                    friendly_name: None,
                });
            }
        }

        Err(ErrorKind::NotFound.into())
    }
}

#[cfg(windows)]
mod windows {
    use super::InterfaceInfo;
    use smoltcp::wire::EthernetAddress;
    use std::{io, mem, ptr};
    use tokio_smoltcp::smoltcp;
    use windows_sys::Win32::{
        Foundation::{ERROR_INSUFFICIENT_BUFFER, NO_ERROR},
        NetworkManagement::{
            IpHelper::{
                ConvertInterfaceIndexToLuid, ConvertInterfaceLuidToAlias, GetIfTable, MIB_IFROW,
                MIB_IFTABLE,
            },
            Ndis::{IF_MAX_STRING_SIZE, NET_LUID_LH},
        },
    };

    fn get_guid(s: &str) -> Option<&str> {
        if let Some(pos) = s.find('{') {
            let p = pos + 1;
            if let Some(end) = s[p..].find('}') {
                return Some(&s[p..(p + end)]);
            }
        }
        return None;
    }

    fn get_friendly_name(index: u32) -> io::Result<String> {
        unsafe {
            let mut luid: NET_LUID_LH = mem::zeroed();
            if ConvertInterfaceIndexToLuid(index, &mut luid as *mut _) == 0 {
                // WCHAR
                let mut name: Vec<u16> = vec![0; IF_MAX_STRING_SIZE as usize + 1];
                if ConvertInterfaceLuidToAlias(&luid as *const _, name.as_mut_ptr(), name.len())
                    == 0
                {
                    return Ok(from_u16(&name)?);
                }
            };
            Err(io::ErrorKind::NotFound.into())
        }
    }

    fn from_u16(s: &[u16]) -> io::Result<String> {
        s.iter()
            .position(|c| *c == 0)
            .map(|pos| String::from_utf16(&s[0..pos]).ok())
            .flatten()
            .ok_or(io::ErrorKind::InvalidData.into())
    }

    pub fn get_interface_info(name: &str) -> io::Result<InterfaceInfo> {
        if let Some(intf_guid) = get_guid(name) {
            let mut size = 0u32;
            let table: *mut MIB_IFTABLE;

            let mut info = InterfaceInfo {
                name: name.to_string(),
                ..Default::default()
            };

            unsafe {
                if GetIfTable(
                    ptr::null_mut::<MIB_IFTABLE>(),
                    &mut size as *mut libc::c_ulong,
                    0,
                ) == ERROR_INSUFFICIENT_BUFFER
                {
                    table = mem::transmute(libc::malloc(size as libc::size_t));
                } else {
                    return Err(io::ErrorKind::NotFound.into());
                }

                if GetIfTable(table, &mut size as *mut libc::c_ulong, 0) == NO_ERROR {
                    let ptr: *const MIB_IFROW = (&(*table).table) as *const _;
                    let table = std::slice::from_raw_parts(ptr, (*table).dwNumEntries as usize);
                    for i in table {
                        let row = &*i;

                        if let Ok(name) = from_u16(&row.wszName) {
                            if let Some(guid) = get_guid(&name) {
                                if guid == intf_guid {
                                    if row.dwPhysAddrLen == 6 {
                                        info.ethernet_address =
                                            EthernetAddress::from_bytes(&row.bPhysAddr[0..6]);
                                    } else {
                                        continue;
                                    }
                                    if row.dwDescrLen > 0 {
                                        if let Ok(desc) = String::from_utf8(
                                            row.bDescr[..(row.dwDescrLen - 1) as usize].to_vec(),
                                        ) {
                                            info.description = Some(desc);
                                        }
                                    }
                                    if let Ok(friendly_name) = get_friendly_name(row.dwIndex) {
                                        info.friendly_name = Some(friendly_name);
                                    }
                                    return Ok(info);
                                }
                            }
                        }
                    }
                }
                libc::free(mem::transmute(table));
            }
        }
        Err(io::ErrorKind::NotFound.into())
    }
}
