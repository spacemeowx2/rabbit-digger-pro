pub fn local_string() -> String {
    "local".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_local_string() {
        assert_eq!(local_string(), "local");
    }
}
