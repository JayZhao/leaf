use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr};

use anyhow::{anyhow, Result};
use tokio::sync::RwLock;
use tracing::{info, error};
use trust_dns_proto::op::{
    header::MessageType, op_code::OpCode, response_code::ResponseCode, Message,
};
use trust_dns_proto::rr::{
    dns_class::DNSClass, rdata, record_data::RData, record_type::RecordType, resource::Record,
};

#[derive(Debug)]
pub enum FakeDnsMode {
    Include,
    Exclude,
}

pub struct FakeDns(RwLock<FakeDnsImpl>);

impl FakeDns {
    pub fn new(mode: FakeDnsMode) -> Self {
        Self(RwLock::new(FakeDnsImpl::new(mode)))
    }

    pub async fn add_filter(&self, filter: String) {
        self.0.write().await.add_filter(filter)
    }

    pub async fn query_domain(&self, ip: &IpAddr) -> Option<String> {
        self.0.read().await.query_domain(ip)
    }

    pub async fn query_fake_ip(&self, domain: &str) -> Option<IpAddr> {
        self.0.read().await.query_fake_ip(domain)
    }

    pub async fn generate_fake_response(&self, request: &[u8]) -> Result<Vec<u8>> {
        self.0.write().await.generate_fake_response(request)
    }

    pub async fn is_fake_ip(&self, ip: &IpAddr) -> bool {
        self.0.read().await.is_fake_ip(ip)
    }
}

struct FakeDnsImpl {
    ip_to_domain: HashMap<u32, String>,
    domain_to_ip: HashMap<String, u32>,
    cursor: u32,
    min_cursor: u32,
    max_cursor: u32,
    ttl: u32,
    filters: Vec<String>,
    mode: FakeDnsMode,
}

impl FakeDnsImpl {
    pub(self) fn new(mode: FakeDnsMode) -> Self {
        let min_cursor = Self::ip_to_u32(&Ipv4Addr::new(198, 18, 0, 0));
        let max_cursor = Self::ip_to_u32(&Ipv4Addr::new(198, 18, 255, 255));
        info!("[FakeDNS] 初始化 | 模式: {:?} | IP范围: 198.18.0.0 - 198.18.255.255", mode);
        Self {
            ip_to_domain: HashMap::new(),
            domain_to_ip: HashMap::new(),
            cursor: min_cursor,
            min_cursor,
            max_cursor,
            ttl: 1,
            filters: Vec::new(),
            mode,
        }
    }

    pub(self) fn add_filter(&mut self, filter: String) {
        info!("[FakeDNS] 添加过滤规则: {}", filter);
        self.filters.push(filter);
    }

    pub(self) fn query_domain(&self, ip: &IpAddr) -> Option<String> {
        let ip = match ip {
            IpAddr::V4(ip) => ip,
            _ => {
                info!("[FakeDNS] 查询域名失败: 不支持的IP类型 {:?}", ip);
                return None;
            }
        };
        let result = self.ip_to_domain.get(&Self::ip_to_u32(ip)).cloned();
        info!("[FakeDNS] 查询域名 | IP: {} | 结果: {:?}", ip, result);
        result
    }

    pub(self) fn query_fake_ip(&self, domain: &str) -> Option<IpAddr> {
        let result = self.domain_to_ip
            .get(domain)
            .map(|v| IpAddr::V4(Self::u32_to_ip(v.to_owned())));
        info!("[FakeDNS] 查询假IP | 域名: {} | 结果: {:?}", domain, result);
        result
    }

    pub(self) fn generate_fake_response(&mut self, request: &[u8]) -> Result<Vec<u8>> {
        let req = match Message::from_vec(request) {
            Ok(req) => req,
            Err(e) => {
                error!("[FakeDNS] DNS请求解析失败: {}", e);
                return Err(anyhow!("DNS请求解析失败: {}", e));
            }
        };

        if req.queries().is_empty() {
            error!("[FakeDNS] DNS请求中没有查询内容");
            return Err(anyhow!("no queries in this DNS request"));
        }

        let query = &req.queries()[0];
        info!("[FakeDNS] 收到DNS查询 | 类型: {:?} | 类: {:?}", query.query_type(), query.query_class());

        if query.query_class() != DNSClass::IN {
            error!("[FakeDNS] 不支持的查询类: {}", query.query_class());
            return Err(anyhow!("unsupported query class {}", query.query_class()));
        }

        let t = query.query_type();
        if t != RecordType::A && t != RecordType::AAAA && t != RecordType::HTTPS {
            error!("[FakeDNS] 不支持的记录类型: {:?}", t);
            return Err(anyhow!("unsupported query record type {:?}", t));
        }

        let raw_name = query.name();
        let domain = if raw_name.is_fqdn() {
            let fqdn = raw_name.to_ascii();
            fqdn[..fqdn.len() - 1].to_string()
        } else {
            raw_name.to_ascii()
        };

        info!("[FakeDNS] 处理域名: {}", domain);

        if !self.accept(&domain) {
            error!("[FakeDNS] 域名未被接受: {}", domain);
            return Err(anyhow!("domain {} not accepted", domain));
        }

        let ip = if let Some(ip) = self.query_fake_ip(&domain) {
            match ip {
                IpAddr::V4(a) => a,
                _ => {
                    error!("[FakeDNS] 意外的IPv6假IP");
                    return Err(anyhow!("unexpected Ipv6 fake IP"));
                }
            }
        } else {
            let ip = self.allocate_ip(&domain)?;
            info!("[FakeDNS] 为域名分配新IP | 域名: {} | IP: {}", domain, ip);
            ip
        };

        let mut resp = Message::new();
        resp.set_id(req.id())
            .set_message_type(MessageType::Response)
            .set_op_code(req.op_code());

        if resp.op_code() == OpCode::Query {
            resp.set_recursion_desired(req.recursion_desired())
                .set_checking_disabled(req.checking_disabled());
        }
        resp.set_response_code(ResponseCode::NoError);
        if !req.queries().is_empty() {
            resp.add_query(query.clone());
        }

        if query.query_type() == RecordType::A {
            let mut ans = Record::new();
            ans.set_name(raw_name.clone())
                .set_rr_type(RecordType::A)
                .set_ttl(self.ttl)
                .set_dns_class(DNSClass::IN)
                .set_data(Some(RData::A(rdata::A(ip))));
            resp.add_answer(ans);
            info!("[FakeDNS] 生成DNS应答 | 域名: {} | IP: {} | TTL: {}", domain, ip, self.ttl);
        }

        Ok(resp.to_vec()?)
    }

    pub(self) fn is_fake_ip(&self, ip: &IpAddr) -> bool {
        let ip = match ip {
            IpAddr::V4(ip) => ip,
            _ => return false,
        };
        let ip = Self::ip_to_u32(ip);
        ip >= self.min_cursor && ip <= self.max_cursor
    }

    fn allocate_ip(&mut self, domain: &str) -> Result<Ipv4Addr> {
        if let Some(prev_domain) = self.ip_to_domain.insert(self.cursor, domain.to_owned()) {
            info!("[FakeDNS] IP重用 | 旧域名: {} | 新域名: {} | IP: {}", 
                prev_domain, domain, Self::u32_to_ip(self.cursor));
            self.domain_to_ip.remove(&prev_domain);
        }
        self.domain_to_ip.insert(domain.to_owned(), self.cursor);
        let ip = Self::u32_to_ip(self.cursor);
        self.prepare_next_cursor()?;
        info!("[FakeDNS] 分配IP | 域名: {} | IP: {}", domain, ip);
        Ok(ip)
    }

    // Make sure `self.cursor` is valid and can be used immediately for next fake IP.
    fn prepare_next_cursor(&mut self) -> Result<()> {
        for _ in 0..3 {
            self.cursor += 1;
            if self.cursor > self.max_cursor {
                self.cursor = self.min_cursor;
            }
            // avoid network and broadcast addresses
            match Self::u32_to_ip(self.cursor).octets()[3] {
                0 | 255 => {
                    continue;
                }
                _ => return Ok(()),
            }
        }
        Err(anyhow!("unable to prepare next cursor"))
    }

    fn accept(&self, domain: &str) -> bool {
        let result = match self.mode {
            FakeDnsMode::Exclude => {
                for d in &self.filters {
                    if domain.contains(d) || d == "*" {
                        info!("[FakeDNS] 域名被排除 | 域名: {} | 匹配规则: {}", domain, d);
                        return false;
                    }
                }
                true
            }
            FakeDnsMode::Include => {
                for d in &self.filters {
                    if domain.contains(d) || d == "*" {
                        info!("[FakeDNS] 域名被包含 | 域名: {} | 匹配规则: {}", domain, d);
                        return true;
                    }
                }
                false
            }
        };
        info!("[FakeDNS] 域名检查结果 | 域名: {} | 模式: {:?} | 结果: {}", domain, self.mode, result);
        result
    }

    fn u32_to_ip(ip: u32) -> Ipv4Addr {
        Ipv4Addr::from(ip)
    }

    fn ip_to_u32(ip: &Ipv4Addr) -> u32 {
        u32::from_be_bytes(ip.octets())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    #[test]
    fn test_u32_to_ip() {
        let ip1 = Ipv4Addr::new(127, 0, 0, 1);
        let ip2 = FakeDnsImpl::u32_to_ip(2130706433u32);
        assert_eq!(ip1, ip2);
    }

    #[test]
    fn test_ip_to_u32() {
        let ip = Ipv4Addr::new(127, 0, 0, 1);
        let ip1 = FakeDnsImpl::ip_to_u32(&ip);
        let ip2 = 2130706433u32;
        assert_eq!(ip1, ip2);
    }
}
