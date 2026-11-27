//! Where a request comes from, as the limits that count by client name it.

use std::net::{IpAddr, Ipv6Addr};

/// The source of a client's address, by which the limits that count by client count it: an
/// IPv4 address, or the /64 network of an IPv6 address, as one machine, home or office
/// chooses its addresses within a /64 at will. An IPv4 address mapped into IPv6 is the IPv4
/// address; text that is no address stays as it is.
pub fn client_source(ip: &str) -> String {
    match ip.parse::<IpAddr>() {
        Ok(IpAddr::V4(v4)) => v4.to_string(),
        Ok(IpAddr::V6(v6)) => match v6.to_ipv4_mapped() {
            Some(v4) => v4.to_string(),
            None => {
                let network = Ipv6Addr::from(u128::from(v6) & (u128::MAX << 64));
                format!("{network}/64")
            }
        },
        Err(_) => ip.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_source_is_an_ipv4_address_or_the_64_of_an_ipv6_one() {
        assert_eq!(client_source("198.51.100.7"), "198.51.100.7");
        assert_eq!(client_source("::ffff:198.51.100.7"), "198.51.100.7");
        assert_eq!(
            client_source("2001:db8:1:2:aaaa:bbbb:cccc:dddd"),
            "2001:db8:1:2::/64"
        );
        assert_eq!(
            client_source("2001:db8:1:2::1"),
            client_source("2001:db8:1:2:ffff:ffff:ffff:ffff")
        );
        assert_ne!(
            client_source("2001:db8:1:2::1"),
            client_source("2001:db8:1:3::1")
        );
        assert_eq!(client_source("::1"), "::/64");
        assert_eq!(client_source("unknown"), "unknown");
    }
}
