use std::fs::File;
use std::io::Read;
use lazy_static::lazy_static;
use std::sync::Arc;
use tracing::{info, error};

pub struct DomainRule {
    binary_domains: Vec<u128>,
}

impl DomainRule {
    /// 获取域名的可注册部分
    /// 
    /// 规则:
    /// 1. 处理特殊的中国相关顶级域名，如 .com.cn, .net.cn 等
    /// 2. 处理常见的二级域名，如 .com, .net 等
    /// 3. 如果不在已知列表中，保持原样返回
    /// 
    /// 示例:
    /// - www.example.com.cn -> example.com.cn
    /// - sub.example.com -> example.com
    /// - example.cn -> example.cn
    /// - www.example.co.uk -> example.co.uk (国外特殊域名也一并处理)
    fn get_registrable_domain(domain: &str) -> String {
        // 如果域名以 www. 开头，去掉它
        let domain = if domain.starts_with("www.") {
            &domain[4..]
        } else {
            domain
        };

        let parts: Vec<&str> = domain.split('.').collect();
        if parts.len() < 2 {
            return domain.to_string();
        }

        // 特殊的三级域名后缀
        const SPECIAL_SUFFIXES: [&str; 14] = [
            "com.cn", "net.cn", "org.cn", "gov.cn", 
            "edu.cn", "mil.cn", "ac.cn", "ah.cn",
            "bj.cn", "sh.cn", "tj.cn", "hz.cn",
            "co.uk", "co.jp"  // 附加一些常见的国外特殊后缀
        ];

        // 常见的二级域名后缀
        const COMMON_SUFFIXES: [&str; 12] = [
            "cn", "com", "net", "org", "edu",
            "gov", "mil", "biz", "info", "pro",
            "name", "xyz"
        ];
        
        // 1. 检查是否是特殊的三级域名
        if parts.len() >= 3 {
            let possible_special = parts[parts.len()-2..].join(".");
            if SPECIAL_SUFFIXES.contains(&possible_special.as_str()) {
                return if parts.len() == 3 {
                    domain.to_string()
                } else {
                    format!("{}.{}", parts[parts.len()-3], possible_special)
                };
            }
        }

        // 2. 检查是否是普通的二级域名
        if COMMON_SUFFIXES.contains(&parts.last().unwrap()) {
            return if parts.len() == 2 {
                domain.to_string()
            } else {
                format!("{}.{}", parts[parts.len()-2], parts.last().unwrap())
            };
        }

        // 3. 如果不在已知列表中，返回原始域名
        domain.to_string()
    }

    /// 将域名转换为用于二分查找的 u128 值
    fn domain_to_u128(domain: &str) -> u128 {
        // 首先获取可注册域名部分
        let domain = Self::get_registrable_domain(domain);
        
        if domain.is_empty() {
            return 0;
        }

        let mut bytes = [0u8; 16];
        let domain_bytes = domain.as_bytes();
        let len = domain_bytes.len().min(16);
        bytes[..len].copy_from_slice(&domain_bytes[domain_bytes.len().saturating_sub(16)..]);

        u128::from_le_bytes(bytes)
    }

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

        info!("📂 Attempting to load binary file: {}", binary_path.display());
        
        // 检查文件是否存在
        if !binary_path.exists() {
            error!("❌ Binary file not found: {}", binary_path.display());
            return Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("Binary file not found: {}", binary_path.display())
            ));
        }
        
        // 加载二进制域名数据
        let mut binary_file = File::open(binary_path)?;
        let mut binary_data = Vec::new();
        binary_file.read_to_end(&mut binary_data)?;

        if binary_data.is_empty() {
            error!("❌ Binary data is empty");
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "Binary data is empty"
            ));
        }
        
        // 确保数据长度是16的倍数
        if binary_data.len() % 16 != 0 {
            error!("❌ Binary data length ({}) is not a multiple of 16", binary_data.len());
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
            error!("⚠️ binary domains are not sorted!");
            info!("🔄 Sorting binary domains...");
            binary_domains.sort_unstable();
        }
            
        info!("\n🔍 First 10 domain entries (decimal and hex):");
        for (i, value) in binary_domains.iter().take(10).enumerate() {
            info!("  [{:2}] {} (hex: 0x{:x})", i, value, value);
            // 尝试将值转换回字符串看看是什么
            let bytes = value.to_le_bytes();
            if let Ok(s) = String::from_utf8(bytes.to_vec()) {
                info!("       ASCII: {}", s);
            }
        }
        
        Ok(Self {
            binary_domains,
        })
    }
    
    /// 检查是否是需要排除的国家顶级域名
    fn is_excluded_country_tld(domain: &str) -> bool {
        const EXCLUDED_COUNTRY_TLDS: [&str; 24] = [
            // 亚洲地区
            ".hk",   // 香港
            ".tw",   // 台湾
            ".sg",   // 新加坡
            ".jp",   // 日本
            ".kr",   // 韩国
            ".in",   // 印度
            ".th",   // 泰国
            ".vn",   // 越南
            ".my",   // 马来西亚
            ".id",   // 印度尼西亚
            ".io",   // 英属印度洋领地
            
            // 欧洲地区
            ".uk",   // 英国
            ".de",   // 德国
            ".fr",   // 法国
            ".it",   // 意大利
            ".es",   // 西班牙
            ".nl",   // 荷兰
            ".ru",   // 俄罗斯
            
            // 美洲地区
            ".us",   // 美国
            ".ca",   // 加拿大
            ".mx",   // 墨西哥
            ".br",   // 巴西
            
            // 大洋洲
            ".au",   // 澳大利亚
            ".nz",   // 新西兰
        ];

        EXCLUDED_COUNTRY_TLDS.iter().any(|&tld| domain.ends_with(tld))
    }
    
    pub fn is_match(&self, domain: &str) -> bool {
        // 首先检查是否是需要排除的国家顶级域名
        if Self::is_excluded_country_tld(domain) {
            info!("❌ Domain '{}' excluded due to country TLD", domain);
            return false;
        }

        // 如果是以 .cn 结尾的域名，则直接返回 true
        if domain.ends_with(".cn") {
            info!("✅ Domain '{}' matched in .cn suffix", domain);
            return true;
        }

        // 2. 检查二进制域名列表
        let domain_value = Self::domain_to_u128(domain);

        // 使用二分查找
        if self.binary_domains.binary_search(&domain_value).is_ok() {
            info!("✅ Domain '{}' matched in binary domain list (value: {})", domain, domain_value);
            return true;
        }

        info!("❌ Domain '{}' did not match any rules", domain);
        false
    }
}

lazy_static! {
    pub static ref SMART_MATCHER: Arc<DomainRule> = {
        match DomainRule::new() {
            Ok(rule) => Arc::new(rule),
            Err(e) => {
                error!("❌ Failed to initialize SMART_MATCHER: {}", e);
                panic!("Failed to initialize SMART_MATCHER: {}", e);
            }
        }
    };
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
        
        info!("\n🚀 Testing DomainRule with:");
        info!("  Binary file: {}", binary_path.display());
        
        let matcher = match DomainRule::new() {
            Ok(m) => m,
            Err(e) => {
                error!("❌ Failed to create DomainRule: {}", e);
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
        
        info!("\n🧪 Testing domain matching:");
        for (domain, expected) in test_cases {
            let result = matcher.is_match(domain);
            info!("  Testing '{}': {} (expected: {})", 
                domain, result, expected);
            assert_eq!(result, expected, 
                "Domain '{}' matching failed", domain);
        }

        // 添加国家顶级域名的测试用例
        let country_tld_cases = vec![
            ("example.hk", false),
            ("example.tw", false),
            ("example.sg", false),
            ("example.jp", false),
            ("example.uk", false),
            ("example.us", false),
            ("example.cn", true),      // 中国域名应该返回 true
            ("example.com.cn", true),  // 中国域名应该返回 true
        ];

        for (domain, expected) in country_tld_cases {
            let result = matcher.is_match(domain);
            info!("  Testing country TLD '{}': {} (expected: {})", 
                domain, result, expected);
            assert_eq!(result, expected, 
                "Country TLD domain '{}' matching failed", domain);
        }
    }

    #[test]
    fn test_get_registrable_domain() {
        let test_cases = vec![
            // 中国特殊域名测试
            ("www.example.com.cn", "example.com.cn"),
            ("sub.example.com.cn", "example.com.cn"),
            ("www.example.net.cn", "example.net.cn"),
            ("www.example.org.cn", "example.org.cn"),
            ("www.example.gov.cn", "example.gov.cn"),
            ("www.dept.edu.cn", "dept.edu.cn"),
            
            // 普通二级域名测试
            ("www.example.com", "example.com"),
            ("sub.example.com", "example.com"),
            ("www.example.net", "example.net"),
            ("www.example.org", "example.org"),
            
            // 国外特殊域名测试
            ("www.example.co.uk", "example.co.uk"),
            ("sub.example.co.uk", "example.co.uk"),
            ("www.example.co.jp", "example.co.jp"),
            
            // 未知后缀测试
            ("example.unknown", "example.unknown"),
            ("sub.example.unknown", "sub.example.unknown"),
            ("t2.xiaohongshu.com", "xiaohongshu.com"),
            
            // 边界情况测试
            ("example", "example"),
            ("com.cn", "com.cn"),
            ("cn", "cn"),
        ];

        for (input, expected) in test_cases {
            let result = DomainRule::get_registrable_domain(input);
            assert_eq!(
                result, expected,
                "Domain '{}' should return '{}' but got '{}'",
                input, expected, result
            );
        }
    }
}

