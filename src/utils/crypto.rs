/// Compare two byte strings in time independent of their contents.
///
/// Lengths leak (the loop runs over the longer input), which is acceptable
/// for the fixed-shape secrets this guards (API keys, RPC passwords): the
/// secret is the content, not its length. Never short-circuits on the first
/// differing byte, unlike `==` on strings.
pub fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    let max_len = a.len().max(b.len());
    if max_len == 0 {
        return true;
    }
    let mut diff = if a.len() == b.len() { 0u8 } else { 1u8 };
    for i in 0..max_len {
        let x = if i < a.len() { a[i] } else { 0 };
        let y = if i < b.len() { b[i] } else { 0 };
        diff |= x ^ y;
    }
    diff == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constant_time_eq_basic() {
        assert!(constant_time_eq(b"abc", b"abc"));
        assert!(!constant_time_eq(b"abc", b"abd"));
        assert!(!constant_time_eq(b"abc", b"abcd"));
        assert!(!constant_time_eq(b"abcd", b"abc"));
        assert!(constant_time_eq(b"", b""));
        assert!(!constant_time_eq(b"", b"a"));
    }
}
