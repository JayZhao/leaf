use std::fs::File;
use std::io::Read;
use std::collections::HashSet;
use protobuf::Message;
use super::geosite::{SiteGroupList, domain::Type};
use protobuf::EnumOrUnknown;
use regex::Regex;

pub struct DomainRule {
    binary_domains: Vec<u128>,
    full_domains: HashSet<String>,
    regex_patterns: Vec<Regex>,
}

impl DomainRule {
    pub fn new() -> std::io::Result<Self> {
        let exe_path = std::env::current_exe()?;
        let exe_dir = if cfg!(test) {
            exe_path.parent()
                .ok_or_else(|| std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    "Could not determine deps directory"
                ))?
                .parent()
                .ok_or_else(|| std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    "Could not determine debug directory"
                ))?
                .to_path_buf()
        } else {
            exe_path.parent()
                .ok_or_else(|| std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    "Could not determine executable directory"
                ))?
                .to_path_buf()
        };

        let binary_path = exe_dir.join("site_cn_binary.dat");
        let other_path = exe_dir.join("site_cn_other.dat");

        println!("📂 Attempting to load binary file: {}", binary_path.display());
        
        // 检查文件是否存在
        if !binary_path.exists() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("Binary file not found: {}", binary_path.display())
            ));
        }
        
        // 加载二进制域名数据
        let mut binary_file = File::open(binary_path)?;
        let mut binary_data = Vec::new();
        binary_file.read_to_end(&mut binary_data)?;
        
        println!("📊 Binary data size: {} bytes", binary_data.len());
        
        // 确保数据长度是16的倍数
        if binary_data.len() % 16 != 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("Binary data length ({}) is not a multiple of 16", binary_data.len())
            ));
        }
        
        // 将二进制数据转换为 u128
        let mut binary_domains: Vec<u128> = binary_data
            .chunks_exact(16)
            .map(|chunk| {
                let mut bytes = [0u8; 16];
                bytes.copy_from_slice(chunk);
                u128::from_le_bytes(bytes)
            })
            .collect();

        // 验证数组是否有序
        let is_sorted = binary_domains.windows(2).all(|w| w[0] <= w[1]);
        if !is_sorted {
            eprintln!("⚠️ Warning: binary domains are not sorted!");
            println!("🔄 Sorting binary domains...");
            binary_domains.sort_unstable();
        }
            
        eprintln!("\n🔍 First 10 domain entries (decimal and hex):");
        for (i, value) in binary_domains.iter().take(10).enumerate() {
            eprintln!("  [{:2}] {} (hex: 0x{:x})", i, value, value);
            // 尝试将值转换回字符串看看是什么
            let bytes = value.to_le_bytes();
            if let Ok(s) = String::from_utf8(bytes.to_vec()) {
                eprintln!("       ASCII: {}", s);
            }
        }
        
        println!("📂 Attempting to load other file: {}", other_path.display());
        
        // 检查其他文件是否存在
        if !other_path.exists() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("Other file not found: {}", other_path.display())
            ));
        }
        
        // 加载其他域名数据
        let mut other_file = File::open(other_path)?;
        let mut other_data = Vec::new();
        other_file.read_to_end(&mut other_data)?;
        
        println!("📊 Other data size: {} bytes", other_data.len());
        
        let other_domains = match SiteGroupList::parse_from_bytes(&other_data) {
            Ok(domains) => domains,
            Err(e) => return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("Failed to parse other domains: {}", e)
            )),
        };
        
        // 从 other_domains 中分离完整域名和正则表达式
        let mut full_domains = HashSet::new();
        let mut regex_domains = Vec::new();

        for site_group in &other_domains.site_group {
            for domain in &site_group.domain {
                if domain.type_ == EnumOrUnknown::new(Type::Full) {
                    full_domains.insert(domain.value.clone());
                } else if domain.type_ == EnumOrUnknown::new(Type::Regex) {
                    regex_domains.push(domain.value.clone());
                }
            }
        }

        // 预编译正则表达式
        let regex_patterns = regex_domains
            .into_iter()
            .filter_map(|pattern| {
                match Regex::new(&pattern) {
                    Ok(regex) => Some(regex),
                    Err(e) => {
                        eprintln!("Warning: Invalid regex pattern '{}': {}", pattern, e);
                        None
                    }
                }
            })
            .collect();

        Ok(Self {
            binary_domains,
            full_domains,
            regex_patterns,
        })
    }
    
    pub fn is_match(&self, domain: &str) -> bool {
        // 1. 检查完整域名匹配
        if self.full_domains.contains(domain) {
            println!("✅ Domain '{}' matched in full domain list", domain);
            return true;
        }

        // 2. 检查二进制域名列表
        let domain_bytes = domain.as_bytes();
        let mut dot_count = 0;
        let mut scan_len = 0;
        let mut start_pos = 0;

        for (i, &byte) in domain_bytes.iter().rev().enumerate() {
            scan_len = i + 1;
            if byte == b'.' {
                dot_count += 1;
                if dot_count == 2 {
                    start_pos = domain_bytes.len() - i;
                    break;
                }
            }
            if scan_len == 16 {
                start_pos = domain_bytes.len() - i;
                break;
            }
        }
        
        let domain_suffix = &domain_bytes[start_pos..];

        let mut padded = [0u8; 16];
        let start = (16 - domain_suffix.len()).max(0);
        padded[start..].copy_from_slice(domain_suffix);
        let domain_value = u128::from_le_bytes(padded);

        // 使用二分查找
        if self.binary_domains.binary_search(&domain_value).is_ok() {
            println!("✅ Domain '{}' matched in binary domain list (value: {})", domain, domain_value);
            return true;
        }

        // 3. 使用正则表达式进行匹配
        for (index, regex) in self.regex_patterns.iter().enumerate() {
            if regex.is_match(domain) {
                println!("✅ Domain '{}' matched by regex pattern #{}: '{}'", 
                    domain, index + 1, regex.as_str());
                return true;
            }
        }

        println!("❌ Domain '{}' did not match any rules", domain);
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    
    fn get_test_file_path(filename: &str) -> PathBuf {
        let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        path.pop();
        path.push(filename);
        path
    }

    #[test]
    fn test_domain_rule() {
        let binary_path = get_test_file_path("site_cn_binary.dat");
        let other_path = get_test_file_path("site_cn_other.dat");
        
        println!("\n🚀 Testing DomainRule with:");
        println!("  Binary file: {}", binary_path.display());
        println!("  Other file: {}", other_path.display());
        
        let matcher = match DomainRule::new() {
            Ok(m) => m,
            Err(e) => {
                panic!("Failed to create DomainRule: {}", e);
            }
        };
        
        // 测试一些域名
        let test_cases = vec![
            // 中国网站 (预期为 true)
            ("www.baidu.com", true),
            ("www.qq.com", true),
            ("www.taobao.com", true),
            ("www.tmall.com", true),
            ("www.jd.com", true),
            ("www.163.com", true),
            ("www.sina.com.cn", true),
            ("www.weibo.com", true),
            ("www.zhihu.com", true),
            ("www.bilibili.com", true),
            ("www.alipay.com", true),
            ("www.douyin.com", true),
            ("www.toutiao.com", true),
            ("www.xiaohongshu.com", true),
            ("www.douban.com", true),
            ("www.meituan.com", true),
            ("www.dianping.com", true),
            ("www.ctrip.com", true),
            ("www.iqiyi.com", true),
            ("www.youku.com", true),
            ("www.sohu.com", true),
            ("www.360.cn", true),
            ("www.huawei.com", true),
            ("www.mi.com", true),
            ("www.pinduoduo.com", true),

            // 国外网站 (预期为 false)
            ("www.google.com", false),
            ("www.facebook.com", false),
            ("www.youtube.com", false),
            ("www.twitter.com", false),
            ("www.instagram.com", false),
            ("www.amazon.com", false),
            ("www.netflix.com", false),
            ("www.microsoft.com", true),
            ("www.apple.com", true),
            ("www.reddit.com", false),
            ("www.wikipedia.org", false),
            ("www.linkedin.com", false),
            ("www.github.com", false),
            ("www.stackoverflow.com", false),
            ("www.medium.com", false),
            ("www.spotify.com", false),
            ("www.twitch.tv", false),
            ("www.discord.com", false),
            ("www.whatsapp.com", false),
            ("www.telegram.org", false),
            ("www.tiktok.com", false),
            ("www.zoom.us", false),
            ("www.adobe.com", false),
            ("www.dropbox.com", false),
            ("www.paypal.com", false),

            // 随机生成的域名 (预期为 false)
            ("xj8k2p5m4n.com", false),
            ("qw9v7y3h1d.net", false),
            ("rt5f2l8c6b.org", false),
            ("mn4s7w9x2v.com", false),
            ("hy6t3q8k5p.net", false),
            ("zb2n9c4m7j.org", false),
            ("wd5x8l1v6h.com", false),
            ("pg3f7t2y4r.net", false),
            ("kj9b5n8m1c.org", false),
            ("ls4h7w2v9t.com", false),
        ];
        
        println!("\n🧪 Testing domain matching:");
        for (domain, expected) in test_cases {
            let result = matcher.is_match(domain);
            println!("  Testing '{}': {} (expected: {})", 
                domain, result, expected);
            assert_eq!(result, expected, 
                "Domain '{}' matching failed", domain);
        }
    }
}

