pub use ads_rust::acctrie::*;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reexported_core_acctrie_works() {
        let mut trie = AccTrie::new();
        trie.insert(b"alpha".to_vec(), 1).unwrap();

        let result = trie.query(&b"alpha".to_vec(), 1).unwrap();
        match result {
            QueryResult::Exists(proof) => assert!(proof.verify()),
            QueryResult::NotExists(_) => panic!("expected membership proof"),
        }
    }
}
