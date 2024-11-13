# extract_cn_domains.py
import sys
import os
from google.protobuf.internal.decoder import _DecodeVarint32
from google.protobuf.internal.encoder import _EncodeVarint
import geosite_pb2
import time
from collections import Counter
import struct
from pathlib import Path

def format_size(size):
    """格式化文件大小显示"""
    for unit in ['B', 'KB', 'MB', 'GB']:
        if size < 1024.0:
            return f"{size:.2f} {unit}"
        size /= 1024.0
    return f"{size:.2f} TB"

def analyze_dat(file_path):
    """分析dat文件的内容"""
    print(f"\n📖 Analyzing: {file_path}")
    print(f"📊 File size: {format_size(os.path.getsize(file_path))}")
    
    try:
        with open(file_path, 'rb') as f:
            data = f.read()
            site_group_list = geosite_pb2.SiteGroupList()
            site_group_list.ParseFromString(data)
            
        stats = {
            'groups': len(site_group_list.site_group),
            'domains': 0,
            'types': Counter(),
            'attributes': Counter(),
            'tags': []
        }
        
        for group in site_group_list.site_group:
            stats['tags'].append(group.tag)
            stats['domains'] += len(group.domain)
            
            for domain in group.domain:
                stats['types'][geosite_pb2.Domain.Type.Name(domain.type)] += 1
                for attr in domain.attribute:
                    if attr.HasField('bool_value'):
                        stats['attributes'][f"{attr.key}(bool)"] += 1
                    elif attr.HasField('int_value'):
                        stats['attributes'][f"{attr.key}(int)"] += 1
        
        return stats, site_group_list
        
    except Exception as e:
        print(f"❌ Error analyzing file: {e}")
        return None, None

def print_stats(stats):
    """打印统计信息"""
    if not stats:
        return
        
    print("\n📊 File Statistics:")
    print(f"  • Groups: {stats['groups']}")
    print(f"  • Total domains: {stats['domains']}")
    
    print("\n  • Domain types distribution:")
    for type_name, count in stats['types'].most_common():
        print(f"    - {type_name}: {count} ({count/stats['domains']*100:.1f}%)")
    
    if stats['attributes']:
        print("\n  • Attributes distribution:")
        for attr_name, count in stats['attributes'].most_common():
            print(f"    - {attr_name}: {count}")
    
    print("\n  • Tags:")
    for tag in stats['tags']:
        print(f"    - {tag}")

def domain_to_u128(domain: str) -> int:
    """将域名转换为u128格式"""
    truncated = domain[-16:] if len(domain) > 16 else domain
    padded_domain = truncated.rjust(16, '\0')
    domain_bytes = padded_domain.encode('ascii')
    return int.from_bytes(domain_bytes, byteorder='little', signed=False)

def extract_and_split_domains(input_file: str, other_output: str, binary_output: str):
    """提取域名并分别处理"""
    stats, site_group_list = analyze_dat(input_file)
    if not stats:
        raise Exception("Failed to analyze input file")
    
    # 存储suffix类型的域名的u128值和原始域名
    suffix_domains_u128 = []
    suffix_domains_original = []  # 存储原始域名
    regex_domains = []  # 存储regex类型的域名
    other_domains = []  # 存储其他类型的域名
    
    # 硬编码规则 - 需要添加的域名
    additional_domains = [
        'icloud.com',
        'apple-cloudkit.com',
        'com.cn',
        'net.cn',
        'org.cn',
        'edu.cn',
        'gov.cn',
        'mil.cn',
        'ac.cn'
    ]
    
    # 硬编码规则 - 需要排除的域名
    excluded_domains = {
        'googleapis.com',
        'gstatic.com',
        'amazon.com',
        'stackoverflow.com',
    }
    
    # 基础的cn域名正则表达式
    base_cn_regex = r"\.cn$"
    
    # 从原始数据中提取域名
    for site_group in site_group_list.site_group:
        if site_group.tag.lower() in {'cn', 'apple-cn'}:
            for domain in site_group.domain:
                if domain.type == 2:  # Domain type (suffix)
                    if domain.value not in excluded_domains:
                        u128_value = domain_to_u128(domain.value)
                        suffix_domains_u128.append(u128_value)
                        suffix_domains_original.append(domain.value)
                elif domain.type == 1:  # Regex type
                    regex_domains.append(domain.value)
                else:
                    other_domains.append((domain.type, domain.value, 
                        [(attr.key, attr.bool_value if attr.HasField('bool_value') 
                          else attr.int_value if attr.HasField('int_value') 
                          else None) for attr in domain.attribute]))
    
    # 添加硬编码的额外域名
    for domain in additional_domains:
        u128_value = domain_to_u128(domain)
        suffix_domains_u128.append(u128_value)
        suffix_domains_original.append(domain)
    
    # 添加基础cn正则表达式
    regex_domains.append(base_cn_regex)
    
    # 将所有regex类型的域名添加到other_domains（用于写入dat文件）
    for regex in regex_domains:
        other_domains.append((1, regex, []))
    
    # 将原始后缀域名和regex域名保存到文本文件
    txt_output = str(Path(binary_output).with_suffix('.txt'))
    with open(txt_output, 'w', encoding='utf-8') as f:
        # 写入后缀域名
        f.write("# Suffix Domains\n")
        for domain in sorted(suffix_domains_original):
            f.write(f"{domain}\n")
        
        # 写入regex域名（如果有的话）
        if regex_domains:
            f.write("\n# Regex Patterns\n")
            for pattern in sorted(regex_domains):
                f.write(f"{pattern}\n")
    
    # 排序u128数组
    suffix_domains_u128.sort()
    
    # 打印前10个数字
    print("\n📊 First 10 domain u128 values:")
    for i, value in enumerate(suffix_domains_u128[:10]):
        # 将u128值转换回ASCII字符串
        bytes_value = value.to_bytes(16, byteorder='little', signed=False)
        ascii_str = bytes_value.decode('ascii').rstrip('\0')
        print(f"  [{i:2d}] {value} (hex: {hex(value)})")
        print(f"       ASCII: {ascii_str}")
    
    # 将u128数组写入二进制文件（使用little-endian）
    with open(binary_output, 'wb') as f:
        for value in suffix_domains_u128:
            f.write(value.to_bytes(16, byteorder='little', signed=False))
    
    # 将其他类型的域名写入site_cn_other.dat
    write_other_domains(other_domains, other_output)
    
    # 打印统计信息
    print(f"\n📊 Domain Statistics:")
    print(f"  • Total suffix domains processed: {len(suffix_domains_u128)}")
    print(f"  • Total regex patterns: {len(regex_domains)}")  # 新增：显示regex数量
    print(f"  • Total other domains: {len(other_domains)}")
    print(f"  • Binary file size: {format_size(Path(binary_output).stat().st_size)}")
    print(f"  • Other domains file size: {format_size(Path(other_output).stat().st_size)}")
    print(f"  • Domain list saved to: {txt_output}")

def write_other_domains(domains, output_file):
    """将其他类型的域名写入文件"""
    site_group_list = geosite_pb2.SiteGroupList()
    site_group = site_group_list.site_group.add()
    site_group.tag = "cn"
    
    for domain_type, domain_value, attributes in domains:
        domain = site_group.domain.add()
        domain.type = domain_type
        domain.value = domain_value
        for key, value in attributes:
            attr = domain.attribute.add()
            attr.key = key
            if isinstance(value, bool):
                attr.bool_value = value
            elif isinstance(value, int):
                attr.int_value = value
    
    with open(output_file, 'wb') as f:
        f.write(site_group_list.SerializeToString())

def main():
    # 获取脚本所在目录的上一级目录
    script_dir = Path(__file__).parent
    parent_dir = script_dir.parent

    # 设置输入输出文件路径
    input_file = parent_dir / "site.dat"
    other_output = parent_dir / "site_cn_other.dat"
    binary_output = parent_dir / "site_cn_binary.dat"

    try:
        print("\n🚀 Starting domain extraction process...")
        print(f"📂 Input file: {input_file}")
        print(f"📂 Output files will be saved to: {parent_dir}")
        
        extract_and_split_domains(str(input_file), str(other_output), str(binary_output))
        print("\n✨ Process completed successfully!")
        
    except Exception as e:
        print(f"\n❌ Error: {e}")
        sys.exit(1)

if __name__ == "__main__":
    main()