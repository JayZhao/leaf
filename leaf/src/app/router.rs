use std::collections::{HashMap};
use std::sync::Arc;
use std::sync::RwLock;

use anyhow::anyhow;
use anyhow::Result;
use cidr::IpCidr;
use futures::TryFutureExt;
use maxminddb::geoip2::Country;
use maxminddb::Mmap;
use tracing::{debug, info, warn};

use crate::app::SyncDnsClient;
use crate::config;
use crate::session::{Network, Session, SocksAddr};
use crate::config::domain_rule::SMART_MATCHER;

pub trait Condition: Send + Sync + Unpin {
    fn apply(&self, sess: &Session) -> bool;
}

struct Rule {
    target: String,
    condition: Box<dyn Condition>,
}

impl Rule {
    fn new(target: String, condition: Box<dyn Condition>) -> Self {
        Rule { 
            target, 
            condition,
        }
    }
}

impl Condition for Rule {
    fn apply(&self, sess: &Session) -> bool {
        self.condition.apply(sess)
    }
}

struct MmdbMatcher {
    reader: Arc<maxminddb::Reader<Mmap>>,
    country_code: String,
}

impl MmdbMatcher {
    fn new(reader: Arc<maxminddb::Reader<Mmap>>, country_code: String) -> Self {
        debug!("Creating MMDB matcher for country code: {}", country_code);
        MmdbMatcher {
            reader,
            country_code,
        }
    }
}

impl Condition for MmdbMatcher {
    fn apply(&self, sess: &Session) -> bool {
        if !sess.destination.is_domain() {
            if let Some(ip) = sess.destination.ip() {
                if let Ok(country) = self.reader.lookup::<Country>(ip) {
                    if let Some(country) = country.country {
                        if let Some(iso_code) = country.iso_code {
                            if iso_code.to_lowercase() == self.country_code.to_lowercase() {
                                debug!("[{}] matches geoip code [{}]", ip, &self.country_code);
                                return true;
                            }
                        }
                    }
                }
            }
        }
        false
    }
}

struct IpCidrMatcher {
    values: Vec<IpCidr>,
}

impl IpCidrMatcher {
    fn new(ips: &mut [String]) -> Self {
        let mut cidrs = Vec::new();
        for ip in ips.iter_mut() {
            let ip = std::mem::take(ip);
            match ip.parse::<IpCidr>() {
                Ok(cidr) => cidrs.push(cidr),
                Err(err) => {
                    debug!("parsing cidr {} failed: {}", ip, err);
                }
            }
            drop(ip);
        }
        IpCidrMatcher { values: cidrs }
    }
}

impl Condition for IpCidrMatcher {
    fn apply(&self, sess: &Session) -> bool {
        if !sess.destination.is_domain() {
            for cidr in &self.values {
                if let Some(ip) = sess.destination.ip() {
                    if cidr.contains(&ip) {
                        debug!("[{}] matches ip-cidr [{}]", ip, &cidr);
                        return true;
                    }
                }
            }
        }
        false
    }
}

struct InboundTagMatcher {
    values: Vec<String>,
}

impl InboundTagMatcher {
    fn new(tags: &mut [String]) -> Self {
        let mut values = Vec::new();
        for t in tags.iter_mut() {
            values.push(std::mem::take(t));
        }
        Self { values }
    }
}

impl Condition for InboundTagMatcher {
    fn apply(&self, sess: &Session) -> bool {
        for v in &self.values {
            if v == &sess.inbound_tag {
                debug!("[{}] matches inbound tag [{}]", &sess.inbound_tag, v);
                return true;
            }
        }
        false
    }
}

struct NetworkMatcher {
    values: Vec<Network>,
}

impl NetworkMatcher {
    fn new(networks: &mut [String]) -> Self {
        let mut values = Vec::new();
        for net in networks.iter_mut() {
            match std::mem::take(net).to_uppercase().as_str() {
                "TCP" => values.push(Network::Tcp),
                "UDP" => values.push(Network::Udp),
                _ => (),
            }
        }
        Self { values }
    }
}

impl Condition for NetworkMatcher {
    fn apply(&self, sess: &Session) -> bool {
        for v in &self.values {
            if v == &sess.network {
                debug!("[{}] matches network [{}]", &sess.network, v);
                return true;
            }
        }
        false
    }
}

struct PortMatcher {
    condition: Box<dyn Condition>,
}

impl PortMatcher {
    fn new(port_ranges: &[String]) -> Self {
        let mut cond_or = ConditionOr::new();
        for pr in port_ranges.iter() {
            match PortRangeMatcher::new(pr) {
                Ok(m) => cond_or.add(Box::new(m)),
                Err(e) => warn!("failed to add port range matcher: {}", e),
            }
        }
        PortMatcher {
            condition: Box::new(cond_or),
        }
    }
}

impl Condition for PortMatcher {
    fn apply(&self, sess: &Session) -> bool {
        self.condition.apply(sess)
    }
}

struct PortRangeMatcher {
    start: u16,
    end: u16,
}

impl PortRangeMatcher {
    fn new(port_range: &str) -> Result<Self> {
        let parts: Vec<&str> = port_range.split('-').collect();
        if parts.len() != 2 {
            return Err(anyhow!("invalid port range"));
        }
        let start = if let Ok(v) = parts[0].parse::<u16>() {
            v
        } else {
            return Err(anyhow!("invalid port range"));
        };
        let end = if let Ok(v) = parts[1].parse::<u16>() {
            v
        } else {
            return Err(anyhow!("invalid port range"));
        };
        if start > end {
            return Err(anyhow!("invalid port range"));
        }
        Ok(PortRangeMatcher { start, end })
    }
}

impl Condition for PortRangeMatcher {
    fn apply(&self, sess: &Session) -> bool {
        let port = sess.destination.port();
        if port >= self.start && port <= self.end {
            debug!(
                "[{}] matches port range [{}-{}]",
                port, self.start, self.end
            );
            true
        } else {
            false
        }
    }
}

struct ConditionAnd {
    conditions: Vec<Box<dyn Condition>>,
}

impl ConditionAnd {
    fn new() -> Self {
        ConditionAnd {
            conditions: Vec::new(),
        }
    }

    fn add(&mut self, cond: Box<dyn Condition>) {
        self.conditions.push(cond)
    }

    fn is_empty(&self) -> bool {
        self.conditions.len() == 0
    }
}

impl Condition for ConditionAnd {
    fn apply(&self, sess: &Session) -> bool {
        for cond in &self.conditions {
            if !cond.apply(sess) {
                return false;
            }
        }
        true
    }
}

struct ConditionOr {
    conditions: Vec<Box<dyn Condition>>,
}

impl ConditionOr {
    fn new() -> Self {
        ConditionOr {
            conditions: Vec::new(),
        }
    }

    fn add(&mut self, cond: Box<dyn Condition>) {
        self.conditions.push(cond)
    }
}

impl Condition for ConditionOr {
    fn apply(&self, sess: &Session) -> bool {
        for cond in &self.conditions {
            if cond.apply(sess) {
                return true;
            }
        }
        false
    }
}

struct SmartMatcher;

impl SmartMatcher {
    fn new() -> Self {
        SmartMatcher
    }
}

impl Condition for SmartMatcher {
    fn apply(&self, sess: &Session) -> bool {
        if let Some(domain) = sess.destination.domain() {
            if SMART_MATCHER.is_match(domain) {
                debug!("[{}] matches smart rule", domain);
                return true;
            }
        }
        false
    }
}

pub struct Router {
    rules: Vec<Rule>,
    domain_resolve: bool,
    dns_client: SyncDnsClient,
    route_cache: RwLock<HashMap<String, String>>,
}

impl Router {
    fn load_rules(rules: &mut Vec<Rule>, routing_rules: &mut [config::router::Rule]) {
        
        let mut mmdb_readers: HashMap<String, Arc<maxminddb::Reader<Mmap>>> = HashMap::new();

        for rule in routing_rules.iter_mut() {
            // 检查是否是 SMART 规则
            if !rule.domains.is_empty() {
                let domain = &rule.domains[0];
                if domain.type_.enum_value() == Ok(config::router::rule::domain::Type::PLAIN)
                    && domain.value == "_SMART_RULE_"
                {
                    rules.push(Rule::new(
                        rule.target_tag.clone(),
                        Box::new(SmartMatcher::new())
                    ));
                    continue;
                }
            }

            let mut cond_and = ConditionAnd::new();

            if !rule.ip_cidrs.is_empty() {
                cond_and.add(Box::new(IpCidrMatcher::new(&mut rule.ip_cidrs)));
            }

            if !rule.mmdbs.is_empty() {
                for mmdb in rule.mmdbs.iter() {
                    let reader = match mmdb_readers.get(&mmdb.file) {
                        Some(r) => r.clone(),
                        None => match maxminddb::Reader::open_mmap(&mmdb.file) {
                            Ok(r) => {
                                info!("Successfully loaded mmdb file: {}", mmdb.file);
                                let r = Arc::new(r);
                                mmdb_readers.insert(mmdb.file.to_owned(), r.clone());
                                r
                            }
                            Err(e) => {
                                warn!("Failed to open mmdb file {}: {:?}", mmdb.file, e);
                                continue;
                            }
                        },
                    };
                    cond_and.add(Box::new(MmdbMatcher::new(
                        reader,
                        mmdb.country_code.clone(),
                    )));
                }
            }

            if !rule.port_ranges.is_empty() {
                cond_and.add(Box::new(PortMatcher::new(&rule.port_ranges)));
            }

            if !rule.networks.is_empty() {
                cond_and.add(Box::new(NetworkMatcher::new(&mut rule.networks)));
            }

            if !rule.inbound_tags.is_empty() {
                cond_and.add(Box::new(InboundTagMatcher::new(&mut rule.inbound_tags)));
            }

            if !cond_and.is_empty() {
                let tag = std::mem::take(&mut rule.target_tag);
                rules.push(Rule::new(tag, Box::new(cond_and)));
            }
        }
    }

    pub fn new(
        router: &mut protobuf::MessageField<config::Router>,
        dns_client: SyncDnsClient,
    ) -> Self {
        let mut rules: Vec<Rule> = Vec::new();
        let mut domain_resolve = false;
        if let Some(router) = router.as_mut() {
            Self::load_rules(&mut rules, &mut router.rules);
            domain_resolve = router.domain_resolve;
        }
        
        Router {
            rules,
            domain_resolve,
            dns_client,
            route_cache: RwLock::new(HashMap::new()),
        }
    }

    pub fn reload(&mut self, router: &mut protobuf::MessageField<config::Router>) -> Result<()> {
        self.rules.clear();
        if let Some(router) = router.as_mut() {
            Self::load_rules(&mut self.rules, &mut router.rules);
            self.domain_resolve = router.domain_resolve;
        }
        Ok(())
    }

    pub async fn pick_route<'a>(&'a self, sess: &'a Session) -> Result<String> {
        let cache_key = if sess.destination.is_domain() {
            sess.destination.domain()
                .ok_or_else(|| anyhow!("illegal domain name"))?
                .to_string()
        } else if let Some(ip) = sess.destination.ip() {
            ip.to_string()
        } else {
            // Return "Direct" tag for invalid destination addresses
            return Ok("Direct".to_string());
        };
        
        if let Some(target) = self.route_cache.read().unwrap().get(&cache_key) {
            info!("🦜 route cache hit for {} -> {}", &sess.destination, target);
            return Ok(target.clone());
        }

        info!("🦑 picking route for {}:{}", &sess.network, &sess.destination);

        for rule in &self.rules {
            let start = std::time::Instant::now();
            let matched = rule.apply(sess);
            let elapsed = start.elapsed();
            
            if let Some(domain) = sess.destination.domain() {
                debug!(
                    "routing domain [{}] on rule [{}] took {:?}, matched: {}",
                    domain,
                    rule.target,
                    elapsed,
                    matched
                );
            } else if let Some(ip) = sess.destination.ip() {
                debug!(
                    "routing ip [{}] on rule [{}] took {:?}, matched: {}",
                    ip,
                    rule.target, 
                    elapsed,
                    matched
                );
            }

            if matched {
                info!("🎯 matched rule [{}] for [{}]", 
                    rule.target, 
                    sess.destination
                );

                let target = rule.target.clone();
                self.route_cache.write().unwrap().insert(
                    cache_key,
                    target.clone()
                );
                return Ok(target);
            }
        }

        if sess.destination.is_domain() && self.domain_resolve {
            let ips = {
                self.dns_client
                    .read()
                    .await
                    .lookup(
                        sess.destination
                            .domain()
                            .ok_or_else(|| anyhow!("illegal domain name"))?,
                    )
                    .map_err(|e| anyhow!("lookup {} failed: {}", sess.destination.host(), e))
                    .await?
            };
            if !ips.is_empty() {
                let mut new_sess = sess.clone();
                new_sess.destination = SocksAddr::from((ips[0], sess.destination.port()));
                debug!(
                    "re-matching with resolved ip [{}] for [{}]",
                    ips[0],
                    sess.destination.host()
                );
                for rule in &self.rules {
                    if rule.apply(&new_sess) {
                        info!("🎯 matched rule [{}] for resolved IP [{}]", rule.target, new_sess.destination);
                        let target = rule.target.clone();
                        self.route_cache.write().unwrap().insert(
                            cache_key,
                            target.clone()
                        );
                        return Ok(target);
                    }
                }
            }
        }

        // When no rules match, default to "trojan_out" tag
        let default_target = "trojan_out".to_string();
        info!("⚡ no rules matched, using default route [{}] for [{}]", default_target, sess.destination);
        self.route_cache.write().unwrap().insert(
            cache_key,
            default_target.clone()
        );
        Ok(default_target)
    }
}

#[cfg(test)]
mod tests {
    use crate::session::SocksAddr;

    use super::*;

    #[test]
    fn test_port_matcher() {
        let mut sess = Session {
            destination: SocksAddr::Domain("www.google.com".to_string(), 22),
            ..Default::default()
        };

        // test port range
        let m = PortMatcher::new(&vec!["1024-5000".to_string(), "6000-7000".to_string()]);
        sess.destination = SocksAddr::Domain("www.google.com".to_string(), 2000);
        assert!(m.apply(&sess));
        sess.destination = SocksAddr::Domain("www.google.com".to_string(), 5001);
        assert!(!m.apply(&sess));
        sess.destination = SocksAddr::Domain("www.google.com".to_string(), 6001);
        assert!(m.apply(&sess));

        // test single port range
        let m = PortMatcher::new(&vec!["22-22".to_string()]);
        sess.destination = SocksAddr::Domain("www.google.com".to_string(), 22);
        assert!(m.apply(&sess));

        // test invalid port ranges
        let m = PortRangeMatcher::new("22-21");
        assert!(m.is_err());
        let m = PortRangeMatcher::new("22");
        assert!(m.is_err());
        let m = PortRangeMatcher::new("22-");
        assert!(m.is_err());
        let m = PortRangeMatcher::new("-22");
        assert!(m.is_err());
        let m = PortRangeMatcher::new("22-abc");
        assert!(m.is_err());
        let m = PortRangeMatcher::new("22-23-24");
        assert!(m.is_err());
    }
}

