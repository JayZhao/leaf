use std::collections::HashMap;
use tracing::debug;

// Trie 树节点结构
pub struct TrieNode {
    // 标记当前节点是否是一个域名后缀的结尾
    is_end: bool,
    // 子节点映射表,key 是域名部分,value 是子节点
    children: HashMap<String, TrieNode>,
}

impl TrieNode {
    pub fn new() -> Self {
        debug!("创建新的 Trie 节点");
        Self {
            is_end: false,
            children: HashMap::new(),
        }
    }

    // 插入一个域名后缀到 Trie 树
    pub fn insert(&mut self, domain: &str) {
        debug!("开始向 Trie 树插入域名后缀: {}", domain);
        let parts: Vec<&str> = domain.split('.').rev().collect();
        debug!("域名分割后的部分(反转): {:?}", parts);
        
        let mut current = self;
        for (i, part) in parts.iter().enumerate() {
            debug!("处理第 {} 个部分: {}", i + 1, part);
            current = current.children
                .entry(part.to_string())
                .or_insert_with(|| {
                    debug!("创建新的子节点: {}", part);
                    TrieNode::new()
                });
        }
        current.is_end = true;
        debug!("域名后缀 {} 插入完成", domain);
    }

    // 检查一个域名是否匹配任何已存储的后缀
    pub fn matches(&self, domain: &str) -> bool {
        debug!("开始匹配域名: {}", domain);
        let parts: Vec<&str> = domain.split('.').rev().collect();
        debug!("域名分割后的部分(反转): {:?}", parts);
        
        let mut current = self;
        for (i, part) in parts.iter().enumerate() {
            debug!("检查第 {} 个部分: {}", i + 1, part);
            
            if current.is_end {
                debug!("在检查 {} 时发现匹配的后缀", part);
                return true;
            }
            
            match current.children.get(*part) {
                Some(node) => {
                    debug!("找到子节点: {}", part);
                    current = node;
                }
                None => {
                    debug!("未找到子节点 {}, 匹配失败", part);
                    return false;
                }
            }
        }
        
        let matched = current.is_end;
        if matched {
            debug!("完全匹配成功");
        } else {
            debug!("到达末尾但未找到完全匹配");
        }
        matched
    }
}