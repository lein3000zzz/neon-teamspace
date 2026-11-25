//! Which addresses an outgoing webhook may connect to.
//!
//! A webhook's URL is chosen by the person who registers it, and the server connects to it
//! from inside its own network, so a URL that pointed there could reach what the network
//! hides: the database, an admin listener, a cloud provider's metadata service. Every
//! address a URL's host resolves to is put in one of the [`Blocked`] ranges, or none,
//! which makes it public:
//!
//! | Range | IPv4 | IPv6 |
//! |---|---|---|
//! | [`Blocked::Unspecified`] | `0.0.0.0/8` | `::` |
//! | [`Blocked::Loopback`] | `127.0.0.0/8` | `::1` |
//! | [`Blocked::Private`] | `10.0.0.0/8`, `172.16.0.0/12`, `192.168.0.0/16` | `64:ff9b:1::/48`, `fec0::/10` |
//! | [`Blocked::SharedAddressSpace`] | `100.64.0.0/10` (carrier-grade NAT) | |
//! | [`Blocked::LinkLocal`] | `169.254.0.0/16` | `fe80::/10` |
//! | [`Blocked::UniqueLocal`] | | `fc00::/7` |
//! | [`Blocked::Metadata`] | `169.254.169.254`, `169.254.170.2`, `169.254.169.253`, `100.100.100.200`, `192.0.0.192` | `fd00:ec2::254`, `fd00:ec2::253` |
//! | [`Blocked::Multicast`] | `224.0.0.0/4` | `ff00::/8` |
//! | [`Blocked::Broadcast`] | `255.255.255.255` | |
//! | [`Blocked::Reserved`] | `192.0.0.0/24`, `192.0.2.0/24`, `192.88.99.0/24`, `198.18.0.0/15`, `198.51.100.0/24`, `203.0.113.0/24`, `240.0.0.0/4` | everything outside `2000::/3`, and in it `2001::/23`, `2001:db8::/32`, `3fff::/20` |
//!
//! An IPv6 address that carries an IPv4 one is judged as that IPv4 address: mapped
//! (`::ffff:a.b.c.d`), NAT64 (`64:ff9b::a.b.c.d`) and 6to4 (`2002:aabb:ccdd::`). Those
//! old forms nothing routes any more, IPv4-compatible `::a.b.c.d` and Teredo `2001::/32`,
//! are reserved.
//!
//! The setting `webhooks.allow_private` lets webhooks reach the ranges a private network
//! uses ([`Blocked::lifted_by_allow_private`]): loopback, private, shared, link-local and
//! unique-local addresses. The others stay refused whatever it says: nothing listens for a
//! webhook on a metadata service, a multicast group or a reserved range, and `0.0.0.0`
//! reaches the server's own host.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

/// Why an address is not public.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, thiserror::Error)]
pub enum Blocked {
    #[error("an unspecified address")]
    Unspecified,
    #[error("a loopback address")]
    Loopback,
    #[error("a private address")]
    Private,
    #[error("a shared (carrier-grade NAT) address")]
    SharedAddressSpace,
    #[error("a link-local address")]
    LinkLocal,
    #[error("a unique-local address")]
    UniqueLocal,
    #[error("a cloud metadata address")]
    Metadata,
    #[error("a multicast address")]
    Multicast,
    #[error("the broadcast address")]
    Broadcast,
    #[error("a reserved address")]
    Reserved,
}

impl Blocked {
    /// Whether `webhooks.allow_private` lets webhooks reach it: the ranges of a private
    /// network, but never a metadata service, multicast, broadcast, reserved or unspecified
    /// address.
    pub fn lifted_by_allow_private(self) -> bool {
        matches!(
            self,
            Self::Loopback
                | Self::Private
                | Self::SharedAddressSpace
                | Self::LinkLocal
                | Self::UniqueLocal
        )
    }
}

/// Why an address may not be reached, if it may not: `allow_private` is
/// `webhooks.allow_private`.
pub fn check_ip(ip: IpAddr, allow_private: bool) -> Result<(), Blocked> {
    match classify(ip) {
        None => Ok(()),
        Some(blocked) if allow_private && blocked.lifted_by_allow_private() => Ok(()),
        Some(blocked) => Err(blocked),
    }
}

/// The range an address is in, or `None` for a public one.
pub fn classify(ip: IpAddr) -> Option<Blocked> {
    match ip {
        IpAddr::V4(v4) => classify_v4(v4),
        IpAddr::V6(v6) => classify_v6(v6),
    }
}

/// Whether `ip` is in `base/len`.
fn in_v4(ip: Ipv4Addr, base: [u8; 4], len: u32) -> bool {
    let mask = u32::MAX.checked_shl(32 - len).unwrap_or(0);
    u32::from(ip) & mask == u32::from(Ipv4Addr::from(base)) & mask
}

const V4_METADATA: [[u8; 4]; 5] = [
    // AWS, Azure, Google Cloud, DigitalOcean, Oracle and others.
    [169, 254, 169, 254],
    // AWS ECS task metadata.
    [169, 254, 170, 2],
    // AWS's DNS resolver, which answers for private zones.
    [169, 254, 169, 253],
    // Alibaba Cloud.
    [100, 100, 100, 200],
    // Oracle Cloud's legacy endpoint.
    [192, 0, 0, 192],
];

fn classify_v4(ip: Ipv4Addr) -> Option<Blocked> {
    if V4_METADATA.contains(&ip.octets()) {
        return Some(Blocked::Metadata);
    }
    // The broadcast address comes before the reserved range it is in.
    let ranges: [([u8; 4], u32, Blocked); 16] = [
        ([0, 0, 0, 0], 8, Blocked::Unspecified),
        ([10, 0, 0, 0], 8, Blocked::Private),
        ([100, 64, 0, 0], 10, Blocked::SharedAddressSpace),
        ([127, 0, 0, 0], 8, Blocked::Loopback),
        ([169, 254, 0, 0], 16, Blocked::LinkLocal),
        ([172, 16, 0, 0], 12, Blocked::Private),
        ([192, 0, 0, 0], 24, Blocked::Reserved),
        ([192, 0, 2, 0], 24, Blocked::Reserved),
        ([192, 88, 99, 0], 24, Blocked::Reserved),
        ([192, 168, 0, 0], 16, Blocked::Private),
        ([198, 18, 0, 0], 15, Blocked::Reserved),
        ([198, 51, 100, 0], 24, Blocked::Reserved),
        ([203, 0, 113, 0], 24, Blocked::Reserved),
        ([224, 0, 0, 0], 4, Blocked::Multicast),
        ([255, 255, 255, 255], 32, Blocked::Broadcast),
        ([240, 0, 0, 0], 4, Blocked::Reserved),
    ];
    ranges
        .iter()
        .find(|(base, len, _)| in_v4(ip, *base, *len))
        .map(|(_, _, blocked)| *blocked)
}

/// Whether `ip` is in `base/len`.
fn in_v6(ip: Ipv6Addr, base: [u16; 8], len: u32) -> bool {
    let mask = u128::MAX.checked_shl(128 - len).unwrap_or(0);
    u128::from(ip) & mask == u128::from(Ipv6Addr::from(base)) & mask
}

const V6_METADATA: [[u16; 8]; 2] = [
    // AWS's instance metadata service, and its DNS resolver, over IPv6.
    [0xfd00, 0x0ec2, 0, 0, 0, 0, 0, 0x0254],
    [0xfd00, 0x0ec2, 0, 0, 0, 0, 0, 0x0253],
];

fn classify_v6(ip: Ipv6Addr) -> Option<Blocked> {
    let segments = ip.segments();
    if V6_METADATA.contains(&segments) {
        return Some(Blocked::Metadata);
    }
    if ip.is_unspecified() {
        return Some(Blocked::Unspecified);
    }
    if ip.is_loopback() {
        return Some(Blocked::Loopback);
    }
    // Forms that carry an IPv4 address are judged as it.
    if let Some(v4) = ip.to_ipv4_mapped() {
        return classify_v4(v4);
    }
    if in_v6(ip, [0x64, 0xff9b, 0, 0, 0, 0, 0, 0], 96) {
        return classify_v4(low_v4(ip));
    }
    if in_v6(ip, [0x2002, 0, 0, 0, 0, 0, 0, 0], 16) {
        let [a, b] = segments[1].to_be_bytes();
        let [c, d] = segments[2].to_be_bytes();
        return classify_v4(Ipv4Addr::new(a, b, c, d));
    }
    let ranges: [([u16; 8], u32, Blocked); 9] = [
        // IPv4-compatible, deprecated and routed nowhere.
        ([0, 0, 0, 0, 0, 0, 0, 0], 96, Blocked::Reserved),
        ([0x64, 0xff9b, 1, 0, 0, 0, 0, 0], 48, Blocked::Private),
        ([0x2001, 0xdb8, 0, 0, 0, 0, 0, 0], 32, Blocked::Reserved),
        // The IETF's protocol assignments, Teredo's 2001::/32 among them.
        ([0x2001, 0, 0, 0, 0, 0, 0, 0], 23, Blocked::Reserved),
        ([0x3fff, 0, 0, 0, 0, 0, 0, 0], 20, Blocked::Reserved),
        ([0xfc00, 0, 0, 0, 0, 0, 0, 0], 7, Blocked::UniqueLocal),
        ([0xfe80, 0, 0, 0, 0, 0, 0, 0], 10, Blocked::LinkLocal),
        ([0xfec0, 0, 0, 0, 0, 0, 0, 0], 10, Blocked::Private),
        ([0xff00, 0, 0, 0, 0, 0, 0, 0], 8, Blocked::Multicast),
    ];
    if let Some((_, _, blocked)) = ranges.iter().find(|(base, len, _)| in_v6(ip, *base, *len)) {
        return Some(*blocked);
    }
    // Only global unicast is public; the rest of the space is unassigned.
    if in_v6(ip, [0x2000, 0, 0, 0, 0, 0, 0, 0], 3) {
        None
    } else {
        Some(Blocked::Reserved)
    }
}

/// The IPv4 address in the last 32 bits of an IPv6 one.
fn low_v4(ip: Ipv6Addr) -> Ipv4Addr {
    let bits = u128::from(ip);
    Ipv4Addr::from(u32::try_from(bits & u128::from(u32::MAX)).unwrap_or(0))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn class(text: &str) -> Option<Blocked> {
        classify(
            text.parse()
                .unwrap_or_else(|_| panic!("{text} is an address")),
        )
    }

    #[test]
    fn every_ipv4_range_is_classified_at_its_edges() {
        let cases = [
            ("0.0.0.0", Some(Blocked::Unspecified)),
            ("0.255.255.255", Some(Blocked::Unspecified)),
            ("1.0.0.0", None),
            ("9.255.255.255", None),
            ("10.0.0.0", Some(Blocked::Private)),
            ("10.255.255.255", Some(Blocked::Private)),
            ("11.0.0.0", None),
            ("100.63.255.255", None),
            ("100.64.0.0", Some(Blocked::SharedAddressSpace)),
            ("100.127.255.255", Some(Blocked::SharedAddressSpace)),
            ("100.128.0.0", None),
            ("126.255.255.255", None),
            ("127.0.0.0", Some(Blocked::Loopback)),
            ("127.0.0.1", Some(Blocked::Loopback)),
            ("127.255.255.255", Some(Blocked::Loopback)),
            ("128.0.0.0", None),
            ("169.253.255.255", None),
            ("169.254.0.0", Some(Blocked::LinkLocal)),
            ("169.254.255.255", Some(Blocked::LinkLocal)),
            ("169.255.0.0", None),
            ("172.15.255.255", None),
            ("172.16.0.0", Some(Blocked::Private)),
            ("172.31.255.255", Some(Blocked::Private)),
            ("172.32.0.0", None),
            ("192.0.0.0", Some(Blocked::Reserved)),
            ("192.0.0.255", Some(Blocked::Reserved)),
            ("192.0.1.0", None),
            ("192.0.2.0", Some(Blocked::Reserved)),
            ("192.0.2.255", Some(Blocked::Reserved)),
            ("192.88.99.1", Some(Blocked::Reserved)),
            ("192.167.255.255", None),
            ("192.168.0.0", Some(Blocked::Private)),
            ("192.168.255.255", Some(Blocked::Private)),
            ("192.169.0.0", None),
            ("198.17.255.255", None),
            ("198.18.0.0", Some(Blocked::Reserved)),
            ("198.19.255.255", Some(Blocked::Reserved)),
            ("198.20.0.0", None),
            ("198.51.100.7", Some(Blocked::Reserved)),
            ("203.0.113.7", Some(Blocked::Reserved)),
            ("203.0.114.0", None),
            ("223.255.255.255", None),
            ("224.0.0.0", Some(Blocked::Multicast)),
            ("239.255.255.255", Some(Blocked::Multicast)),
            ("240.0.0.0", Some(Blocked::Reserved)),
            ("254.255.255.255", Some(Blocked::Reserved)),
            ("255.255.255.254", Some(Blocked::Reserved)),
            ("255.255.255.255", Some(Blocked::Broadcast)),
            ("8.8.8.8", None),
            ("93.184.215.14", None),
        ];
        for (address, want) in cases {
            assert_eq!(class(address), want, "{address}");
        }
    }

    #[test]
    fn metadata_services_are_named_apart_from_their_ranges() {
        for address in [
            "169.254.169.254",
            "169.254.170.2",
            "169.254.169.253",
            "100.100.100.200",
            "192.0.0.192",
            "fd00:ec2::254",
            "fd00:ec2::253",
            "::ffff:169.254.169.254",
            "64:ff9b::a9fe:a9fe",
            "2002:a9fe:a9fe::1",
        ] {
            assert_eq!(class(address), Some(Blocked::Metadata), "{address}");
        }
        assert_eq!(class("169.254.169.1"), Some(Blocked::LinkLocal));
        assert_eq!(class("fd00:ec2::1"), Some(Blocked::UniqueLocal));
    }

    #[test]
    fn every_ipv6_range_is_classified() {
        let cases = [
            ("::", Some(Blocked::Unspecified)),
            ("::1", Some(Blocked::Loopback)),
            ("::2", Some(Blocked::Reserved)),
            ("::7f00:1", Some(Blocked::Reserved)),
            ("::808:808", Some(Blocked::Reserved)),
            ("100::1", Some(Blocked::Reserved)),
            ("64:ff9b:1::1", Some(Blocked::Private)),
            ("2001::1", Some(Blocked::Reserved)),
            ("2001:1ff::1", Some(Blocked::Reserved)),
            ("2001:200::1", None),
            ("2001:db8::1", Some(Blocked::Reserved)),
            ("2001:4860:4860::8888", None),
            ("2606:4700:4700::1111", None),
            ("3fff::1", Some(Blocked::Reserved)),
            ("3fff:fff:ffff::1", Some(Blocked::Reserved)),
            ("3fff:1000::1", None),
            ("4000::1", Some(Blocked::Reserved)),
            ("fbff::1", Some(Blocked::Reserved)),
            ("fc00::1", Some(Blocked::UniqueLocal)),
            ("fd12:3456::1", Some(Blocked::UniqueLocal)),
            ("fdff:ffff::1", Some(Blocked::UniqueLocal)),
            ("fe80::1", Some(Blocked::LinkLocal)),
            ("febf::1", Some(Blocked::LinkLocal)),
            ("fec0::1", Some(Blocked::Private)),
            ("feff::1", Some(Blocked::Private)),
            ("ff02::1", Some(Blocked::Multicast)),
            ("ff0e::1", Some(Blocked::Multicast)),
        ];
        for (address, want) in cases {
            assert_eq!(class(address), want, "{address}");
        }
    }

    #[test]
    fn ipv6_forms_that_carry_ipv4_are_judged_as_it() {
        let cases = [
            ("::ffff:127.0.0.1", Some(Blocked::Loopback)),
            ("::ffff:7f00:1", Some(Blocked::Loopback)),
            ("::ffff:10.1.2.3", Some(Blocked::Private)),
            ("::ffff:192.168.1.1", Some(Blocked::Private)),
            ("::ffff:100.64.0.1", Some(Blocked::SharedAddressSpace)),
            ("::ffff:0.0.0.0", Some(Blocked::Unspecified)),
            ("::ffff:224.0.0.1", Some(Blocked::Multicast)),
            ("::ffff:8.8.8.8", None),
            ("64:ff9b::7f00:1", Some(Blocked::Loopback)),
            ("64:ff9b::a00:1", Some(Blocked::Private)),
            ("64:ff9b::808:808", None),
            ("2002:7f00:1::", Some(Blocked::Loopback)),
            ("2002:c0a8:101::1", Some(Blocked::Private)),
            ("2002:808:808::1", None),
        ];
        for (address, want) in cases {
            assert_eq!(class(address), want, "{address}");
        }
    }

    #[test]
    fn allow_private_lifts_a_private_networks_ranges_alone() {
        let lifted = [
            "127.0.0.1",
            "::1",
            "10.0.0.1",
            "172.20.0.5",
            "192.168.1.1",
            "100.64.0.1",
            "169.254.1.1",
            "fe80::1",
            "fd00::1",
            "::ffff:172.17.0.2",
        ];
        for address in lifted {
            let ip: IpAddr = address.parse().expect("an address");
            assert!(check_ip(ip, false).is_err(), "{address} refused by default");
            assert_eq!(check_ip(ip, true), Ok(()), "{address} with allow_private");
        }
        let never = [
            ("0.0.0.0", Blocked::Unspecified),
            ("::", Blocked::Unspecified),
            ("169.254.169.254", Blocked::Metadata),
            ("fd00:ec2::254", Blocked::Metadata),
            ("::ffff:169.254.169.254", Blocked::Metadata),
            ("224.0.0.1", Blocked::Multicast),
            ("ff02::1", Blocked::Multicast),
            ("255.255.255.255", Blocked::Broadcast),
            ("192.0.2.1", Blocked::Reserved),
            ("2001:db8::1", Blocked::Reserved),
            ("::7f00:1", Blocked::Reserved),
        ];
        for (address, want) in never {
            let ip: IpAddr = address.parse().expect("an address");
            assert_eq!(check_ip(ip, true), Err(want), "{address}");
            assert_eq!(check_ip(ip, false), Err(want), "{address}");
        }
        assert_eq!(check_ip("1.1.1.1".parse().unwrap(), false), Ok(()));
        assert_eq!(check_ip("2606:4700::1111".parse().unwrap(), false), Ok(()));
    }

    #[test]
    fn refusals_name_their_range() {
        assert_eq!(Blocked::Loopback.to_string(), "a loopback address");
        assert_eq!(Blocked::Metadata.to_string(), "a cloud metadata address");
    }
}
