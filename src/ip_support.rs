#[cfg(not(any(feature = "ipv4", feature = "ipv6")))]
std::compile_error!("At least one of the features 'ipv4' or 'ipv6' must be enabled.");

use std::sync::OnceLock;

static IP_SUPPORT: OnceLock<IpSupport> = OnceLock::new();

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum IpSupport {
    #[allow(dead_code)]
    V4,
    #[allow(dead_code)]
    V6,
    #[default]
    Both,
}

impl IpSupport {
    pub fn get() -> Self {
        if !cfg!(feature = "ipv4") {
            return IpSupport::V6;
        }
        if !cfg!(feature = "ipv6") {
            return IpSupport::V4;
        }
        IP_SUPPORT.get().copied().unwrap_or(IpSupport::default())
    }

    #[allow(dead_code)]
    pub fn set(&self) {
        if !cfg!(feature = "ipv4") {
            assert!(
                !matches!(self, IpSupport::V4 | IpSupport::Both),
                "IPv4 support is disabled at compile time"
            );
        }
        if !cfg!(feature = "ipv6") {
            assert!(
                !matches!(self, IpSupport::V6 | IpSupport::Both),
                "IPv6 support is disabled at compile time"
            );
        }
        let _ = IP_SUPPORT.set(*self);
    }

    pub fn ipv4() -> bool {
        matches!(Self::get(), IpSupport::V4 | IpSupport::Both) && cfg!(feature = "ipv4")
    }

    pub fn ipv6() -> bool {
        matches!(Self::get(), IpSupport::V6 | IpSupport::Both) && cfg!(feature = "ipv6")
    }
}
