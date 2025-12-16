# acctrie

这是一个用 Rust 实现的简单前缀树（trie）变体：

- 非叶节点：扩展节点（`ExtensionNode`），包含子指针数组（用 `HashMap<char, Box<ExtensionNode>>` 实现）。
- 叶子节点：`LeafNode`，包含 `suffix`、`values`（值集合）、`acc`（累加器）、双向指针 `prev/next` 及 `prev_key/next_key`。
- 累加器：叶子节点的 `acc` 为当前叶子 `values` 和前序叶子的累加和。
- 从根节点到叶节点的路径：`AccTrie::path_to_leaf` 返回每一级的 `key_part`，累加器部分目前以 `None` 占位。

运行测试：

```bash
cargo test
```
