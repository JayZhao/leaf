# extract_cn_domains.py
import sys
import os
from google.protobuf.internal.decoder import _DecodeVarint32
from google.protobuf.internal.encoder import _EncodeVarint
import geosite_pb2
import time
from collections import Counter

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

def extract_cn_domains(input_file):
    """从site.dat中提取cn和apple-cn的域名"""
    start_time = time.time()
    
    # 分析输入文件
    print("\n📌 Analyzing input file...")
    input_stats, site_group_list = analyze_dat(input_file)
    if not input_stats:
        raise Exception("Failed to analyze input file")
    print_stats(input_stats)
    
    cn_domains = set()
    target_tags = {'cn', 'apple-cn'}
    domain_type_count = Counter()
    
    try:
        original_cn_count = 0
        for site_group in site_group_list.site_group:
            if site_group.tag.lower() in target_tags:
                print(f"\n🏷️  Processing tag: {site_group.tag}")
                domain_count = 0
                original_cn_count += len(site_group.domain)
                
                for domain in site_group.domain:
                    domain_type = domain.type
                    domain_value = domain.value
                    type_str = geosite_pb2.Domain.Type.Name(domain_type)
                    domain_type_count[type_str] += 1
                    
                    attributes = [(attr.key, 
                                 attr.bool_value if attr.HasField('bool_value') 
                                 else attr.int_value if attr.HasField('int_value') 
                                 else None) 
                                for attr in domain.attribute]
                    
                    if domain_count < 5:  # 只显示前5个域名作为示例
                        print(f"  💠 {type_str}: {domain_value} {attributes if attributes else ''}")
                    elif domain_count == 5:
                        print("  ... (more domains)")
                    domain_count += 1
                    
                    cn_domains.add((domain_type, domain_value, tuple(attributes)))
                print(f"  ✅ Found {domain_count} domains in {site_group.tag}")

    except Exception as e:
        print(f"❌ Error processing domains: {e}")
        raise

    elapsed_time = time.time() - start_time
    print(f"\n⏱️  Processing completed in {elapsed_time:.2f} seconds")
    print(f"\n📊 Extraction Statistics:")
    print(f"  • Original CN domains: {original_cn_count}")
    print(f"  • Unique domains after merge: {len(cn_domains)}")
    print(f"  • Domain types distribution:")
    for type_name, count in domain_type_count.most_common():
        print(f"    - {type_name}: {count} ({count/len(cn_domains)*100:.1f}%)")
    
    return cn_domains

def write_cn_dat(domains, output_file):
    """将合并后的域名写入新的site_cn.dat文件"""
    start_time = time.time()
    print(f"\n💾 Writing to: {output_file}")
    
    # 创建一个新的SiteGroupList
    site_group_list = geosite_pb2.SiteGroupList()
    site_group = site_group_list.site_group.add()
    site_group.tag = "cn"
    
    for domain_type, domain_value, attributes in sorted(domains):
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

    # 序列化并写入文件
    with open(output_file, 'wb') as f:
        f.write(site_group_list.SerializeToString())
    
    # 验证输出文件
    print("\n📌 Verifying output file...")
    output_stats, _ = analyze_dat(output_file)
    if output_stats:
        print_stats(output_stats)
    
    new_size = os.path.getsize(output_file)
    elapsed_time = time.time() - start_time
    print(f"\n✅ Write completed in {elapsed_time:.2f} seconds")
    
    # 文件大小比较
    original_size = os.path.getsize(sys.argv[1])
    print(f"\n💾 Size comparison:")
    print(f"  • Original: {format_size(original_size)}")
    print(f"  • Extracted: {format_size(new_size)}")
    compression_ratio = (original_size - new_size) / original_size * 100
    print(f"  • Reduction: {compression_ratio:.1f}% ({format_size(original_size - new_size)})")

def main():
    if len(sys.argv) != 3:
        print("Usage: python extract_cn_domains.py <input_site.dat> <output_site_cn.dat>")
        sys.exit(1)

    input_file = sys.argv[1]
    output_file = sys.argv[2]

    try:
        print("\n🚀 Starting domain extraction process...")
        cn_domains = extract_cn_domains(input_file)
        write_cn_dat(cn_domains, output_file)
        print("\n✨ Process completed successfully!")
        
    except Exception as e:
        print(f"\n❌ Error: {e}")
        sys.exit(1)

if __name__ == "__main__":
    main()