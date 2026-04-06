use anyhow::{self as ah, Context as _, format_err as err};

#[derive(Debug, Clone, rkyv::Archive, rkyv::Deserialize, rkyv::Serialize)]
pub struct FixedStr<const N: usize> {
    data: [u8; N],
    len: u16,
}

impl<const N: usize> FixedStr<N> {
    pub fn from_str(s: &str) -> ah::Result<Self> {
        assert!(N <= u16::MAX as usize, "FixedStr: N is too large");
        if s.len() > N {
            return Err(err!("FixedStr: Input string is too long (max {} bytes)", N));
        }
        let mut data = [0u8; N];
        data[..s.len()].copy_from_slice(&s.as_bytes()[..s.len()]);
        Ok(Self {
            len: s.len().try_into().context("FixedStr: Overflow")?,
            data,
        })
    }

    pub fn from_str_trunc(s: &str) -> Self {
        let len = s.len().min(N);
        let s = &s[..len];
        Self::from_str(s).expect("FixedStr: from_str failed.")
    }

    pub fn as_str(&self) -> ah::Result<&str> {
        if self.len as usize > N {
            return Err(err!("FixedStr: Invalid length {} (max {})", self.len, N));
        }
        str::from_utf8(&self.data[..self.len as usize])
            .map_err(|e| err!("FixedStr: Invalid UTF-8: {e}"))
    }

    pub fn as_str_lossy(&self) -> String {
        let len = N.min(self.len as usize);
        String::from_utf8_lossy(&self.data[..len]).to_string()
    }

    pub fn as_bytes(&self) -> &[u8] {
        let len = N.min(self.len as usize);
        &self.data[..len]
    }
}

impl<const N: usize> Default for FixedStr<N> {
    fn default() -> Self {
        Self {
            len: 0,
            data: [0u8; N],
        }
    }
}

impl<const N: usize> std::fmt::Display for FixedStr<N> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.as_str() {
            Ok(s) => write!(f, "{}", s),
            Err(_) => write!(f, "<FixedStr: Invalid UTF-8>"),
        }
    }
}
