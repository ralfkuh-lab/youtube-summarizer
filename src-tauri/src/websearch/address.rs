//! Adresspruefung und Resolver fuer `fetch_page`: gesperrte Ziele werden
//! erkannt, bevor eine Verbindung entsteht, und DNS-Ergebnisse gefiltert.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, ToSocketAddrs};
use std::sync::Arc;

use reqwest::dns::{Addrs, Name, Resolve, Resolving};
use url::{Host, Url};

use super::WebError;

// --------------------------------------------------------------- Adressen --

/// Sperrt alle Adressen, die nicht oeffentlich erreichbar sein duerfen.
/// Eingebettete IPv4-Adressen (IPv4-mapped, IPv4-compatible, 6to4, NAT64,
/// Teredo) werden derselben IPv4-Pruefung unterzogen.
pub fn is_blocked_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => is_blocked_ipv4(v4),
        IpAddr::V6(v6) => {
            // :: und ::1
            if v6.is_unspecified() || v6.is_loopback() {
                return true;
            }
            let segments = v6.segments();
            if segments[0] & 0xffc0 == 0xfe80 {
                return true; // fe80::/10
            }
            if segments[0] & 0xfe00 == 0xfc00 {
                return true; // fc00::/7
            }
            if segments[0] & 0xff00 == 0xff00 {
                return true; // ff00::/8
            }
            match embedded_ipv4(v6) {
                Some(v4) => is_blocked_ipv4(v4),
                None => false,
            }
        }
    }
}

fn is_blocked_ipv4(ip: Ipv4Addr) -> bool {
    let octets = ip.octets();
    octets[0] == 0 // 0.0.0.0/8
        || octets[0] == 10 // 10/8
        || (octets[0] == 100 && octets[1] & 0xc0 == 64) // 100.64/10
        || octets[0] == 127 // 127/8
        || (octets[0] == 169 && octets[1] == 254) // 169.254/16
        || (octets[0] == 172 && octets[1] & 0xf0 == 16) // 172.16/12
        || (octets[0] == 192 && octets[1] == 168) // 192.168/16
        || octets[0] & 0xf0 == 224 // 224.0.0.0/4
        || octets[0] & 0xf0 == 240 // 240.0.0.0/4 inkl. Broadcast
}

/// Liefert die in einer IPv6-Adresse eingebettete IPv4-Adresse, falls die
/// Adresse einem der bekannten Uebergangsformate angehoert.
fn embedded_ipv4(ip: Ipv6Addr) -> Option<Ipv4Addr> {
    let segments = ip.segments();
    let last32 = || {
        Ipv4Addr::new(
            (segments[6] >> 8) as u8,
            segments[6] as u8,
            (segments[7] >> 8) as u8,
            segments[7] as u8,
        )
    };
    // ::ffff:0:0/96 (IPv4-mapped)
    if segments[0..5] == [0, 0, 0, 0, 0] && segments[5] == 0xffff {
        return Some(last32());
    }
    // ::/96 (IPv4-compatible); :: und ::1 sind bereits gesperrt
    if segments[0..6] == [0, 0, 0, 0, 0, 0] {
        return Some(last32());
    }
    // 2002::/16 (6to4)
    if segments[0] == 0x2002 {
        return Some(Ipv4Addr::new(
            (segments[1] >> 8) as u8,
            segments[1] as u8,
            (segments[2] >> 8) as u8,
            segments[2] as u8,
        ));
    }
    // 64:ff9b::/96 und 64:ff9b:1::/48 (NAT64); eingebettet sind die letzten
    // 32 Bit. Eine konservative Auslegung: beide Praefixe pruefen die letzten
    // 32 Bit als IPv4.
    if segments[0] == 0x0064 && segments[1] == 0xff9b && (segments[2] == 0 || segments[2] == 1) {
        return Some(last32());
    }
    // 2001:0::/32 (Teredo); eingebettetes IPv4 = letzte 32 Bit invertiert
    if segments[0] == 0x2001 && segments[1] == 0 {
        let raw = ((segments[6] as u32) << 16) | segments[7] as u32;
        return Some(Ipv4Addr::from(!raw));
    }
    None
}

/// Ziel einer URL: feste Adresse oder Name, der noch aufgeloest werden muss.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FetchTarget {
    Address(IpAddr),
    Domain(String),
}

/// Prueft Schema, Zugangsdaten und Host einer URL. Bei einer IP-Adresse wird
/// `blocked` sofort angewandt (kein DNS); Domainnamen gehen an den Resolver,
/// der dieselbe Pruefung auf die aufgeloesten Adressen anwendet.
pub(crate) fn check_fetch_url(
    url: &Url,
    blocked: &(dyn Fn(IpAddr) -> bool + Sync),
) -> Result<FetchTarget, WebError> {
    if !matches!(url.scheme(), "http" | "https") {
        return Err(WebError::InvalidUrl);
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(WebError::InvalidUrl);
    }
    match url.host() {
        Some(Host::Ipv4(ip)) => blocked_target(IpAddr::V4(ip), blocked),
        Some(Host::Ipv6(ip)) => blocked_target(IpAddr::V6(ip), blocked),
        Some(Host::Domain(name)) => Ok(FetchTarget::Domain(name.to_string())),
        None => Err(WebError::InvalidUrl),
    }
}

fn blocked_target(
    ip: IpAddr,
    blocked: &(dyn Fn(IpAddr) -> bool + Sync),
) -> Result<FetchTarget, WebError> {
    if blocked(ip) {
        Err(WebError::AddressNotAllowed)
    } else {
        Ok(FetchTarget::Address(ip))
    }
}

/// Filtert gesperrte Adressen aus einem DNS-Ergebnis. Bleibt nichts uebrig, ist
/// der Host nicht erlaubt - verbunden wird nur zu den gefilterten Adressen
/// (schuetzt auch vor DNS-Rebinding).
pub(crate) fn filter_resolved(
    addresses: Vec<SocketAddr>,
    blocked: &(dyn Fn(IpAddr) -> bool + Sync),
) -> Result<Vec<SocketAddr>, WebError> {
    let allowed = addresses
        .into_iter()
        .filter(|address| !blocked(address.ip()))
        .collect::<Vec<_>>();
    if allowed.is_empty() {
        Err(WebError::NoAllowedAddress)
    } else {
        Ok(allowed)
    }
}

/// Resolver, der gesperrte Adressen schon vor dem Verbindungsaufbau ausfiltert.
pub(crate) struct FilteredResolver {
    pub(crate) blocked: Arc<dyn Fn(IpAddr) -> bool + Send + Sync>,
}

impl Resolve for FilteredResolver {
    fn resolve(&self, name: Name) -> Resolving {
        let blocked = self.blocked.clone();
        let host = name.as_str().to_string();
        Box::pin(async move {
            let looked_up = tokio::task::spawn_blocking(move || {
                (host.as_str(), 0u16)
                    .to_socket_addrs()
                    .map(|addresses| addresses.collect::<Vec<SocketAddr>>())
            })
            .await
            .map_err(boxed_error)?
            .map_err(boxed_error)?;
            let allowed = filter_resolved(looked_up, blocked.as_ref()).map_err(boxed_error)?;
            Ok(Box::new(allowed.into_iter()) as Addrs)
        })
    }
}

type BoxedError = Box<dyn std::error::Error + Send + Sync>;

fn boxed_error(error: impl std::error::Error + Send + Sync + 'static) -> BoxedError {
    Box::new(error)
}
