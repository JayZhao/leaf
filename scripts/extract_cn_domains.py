import sys
import os
from pathlib import Path
from typing import List, Set, Tuple, Any, Counter
from google.protobuf.internal.decoder import _DecodeVarint32
from google.protobuf.internal.encoder import _EncodeVarint
import geosite_pb2
from publicsuffix2 import get_sld
import sys
from pathlib import Path

# 常量定义
ADDITIONAL_DOMAINS = [
    'icloud.com',
    'apple-cloudkit.com',
    'wofhwifhafalffagy.com',
    'appstoreconnect.apple.com',
]

EXCLUDED_DOMAINS = {
    'googleapis.com',
    'gstatic.com',
    'amazon.com',
    'stackoverflow.com',
    'adobe.com',
}

# 如果顶级域名部分是代表国家的,比如sg,tw,hk等等,则跳过, 因为这些域名通常是国外的, 一旦代表国家的时候只有cn可以被接受
EXCLUDED_COUNTRY_TLDS = {
    # 亚洲地区
    'hk',   # 香港
    'tw',   # 台湾
    'sg',   # 新加坡
    'jp',   # 日本
    'kr',   # 韩国
    'in',   # 印度
    'th',   # 泰国
    'vn',   # 越南
    'my',   # 马来西亚
    'id',   # 印度尼西亚
    
    # 欧洲地区
    'uk',   # 英国
    'de',   # 德国
    'fr',   # 法国
    'it',   # 意大利
    'es',   # 西班牙
    'nl',   # 荷兰
    'ru',   # 俄罗斯
    
    # 美洲地区
    'us',   # 美国
    'ca',   # 加拿大
    'mx',   # 墨西哥
    'br',   # 巴西
    
    # 大洋洲
    'au',   # 澳大利亚
    'nz',   # 新西兰
}

def format_size(size):
    """格式化文件大小显示"""
    for unit in ['B', 'KB', 'MB', 'GB']:
        if size < 1024.0:
            return f"{size:.2f} {unit}"
        size /= 1024.0
    return f"{size:.2f} TB"

def analyze_dat(file_path):
    """分析dat文件的内容"""
    print(f"\n📖 正在分析: {file_path}")
    print(f"📊 文件大小: {format_size(os.path.getsize(file_path))}")
    
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
        print(f"❌ 文件分析错误: {e}")
        return None, None

def print_stats(stats):
    """打印统计信息"""
    if not stats:
        return
        
    print("\n📊 文件统计:")
    print(f"  • 组数: {stats['groups']}")
    print(f"  • 总域名数: {stats['domains']}")
    
    print("\n  • 域名类型分布:")
    for type_name, count in stats['types'].most_common():
        print(f"    - {type_name}: {count} ({count/stats['domains']*100:.1f}%)")
    
    if stats['attributes']:
        print("\n  • 属性分布:")
        for attr_name, count in stats['attributes'].most_common():
            print(f"    - {attr_name}: {count}")
    
    print("\n  • 标签:")
    for tag in stats['tags']:
        print(f"    - {tag}")

def domain_to_u128(domain: str) -> int:
    """将域名转换为u128格式
    1. 如果域名以www开头，则去掉www
    2. 移除所有非字母数字字符
    3. 取最后16个字符填充到u128中
    """
    # 获取可注册域名
    registrable_domain = get_sld(domain)
    if not registrable_domain:
        registrable_domain = domain
        return None
    
    domain_bytes = bytes(registrable_domain, 'utf-8')
    
    # 取最后16个字节
    domain_suffix = domain_bytes[-16:] if len(domain_bytes) > 16 else domain_bytes
    
    # 创建16字节数组
    bytes_array = bytearray(16)
    # 从左边开始填充
    bytes_array[:len(domain_suffix)] = domain_suffix
    
    result = int.from_bytes(bytes_array, byteorder='little', signed=False)
    
    return result

def extract_and_split_domains(input_file: str, binary_output: str):
    """提取域名并分别处理"""
    stats, site_group_list = analyze_dat(input_file)
    if not stats:
        raise Exception("文件分析失败")
    
    suffix_domains_u128 = []
    suffix_domains_original = []
    
    excluded_domains_u128 = {domain_to_u128(domain) for domain in EXCLUDED_DOMAINS}
    
    for site_group in site_group_list.site_group:
        if site_group.tag.lower() in {'cn', 'apple-cn'}:
            for domain in site_group.domain:
                if domain.type == 2:  # Domain type (suffix)
                    tld = domain.value.split('.')[-1].lower()
                    if tld in EXCLUDED_COUNTRY_TLDS:
                        continue
                        
                    u128_value = domain_to_u128(domain.value)
                    if u128_value not in excluded_domains_u128:
                        suffix_domains_u128.append(u128_value)
                        suffix_domains_original.append(domain.value)
    
    for domain in ADDITIONAL_DOMAINS:
        u128_value = domain_to_u128(domain)
        suffix_domains_u128.append(u128_value)
        suffix_domains_original.append(domain)
    
    txt_output = str(Path(binary_output).with_suffix('.txt'))
    with open(txt_output, 'w', encoding='utf-8') as f:
        f.write("# Suffix Domains\n")
        for domain in sorted(set(suffix_domains_original)):
            f.write(f"{domain}\n")
    
    suffix_domains_u128 = list(set(suffix_domains_u128))
    suffix_domains_u128.sort()
    
    with open(binary_output, 'wb') as f:
        for value in suffix_domains_u128:
            f.write(value.to_bytes(16, byteorder='little', signed=False))
    
    print(f"\n📊 域名统计:")
    print(f"  • 处理的后缀域名总数: {len(suffix_domains_u128)}")
    print(f"  • 二进制文件大小: {format_size(Path(binary_output).stat().st_size)}")
    print(f"  • 域名列表已保存至: {txt_output}")

def main():
    script_dir = Path(__file__).parent
    parent_dir = script_dir.parent

    input_file = "site.dat"

    target_dirs = [
        parent_dir / "target/debug",
        parent_dir / "target/release"
    ]

    try:
        print("\n🚀 开始域名提取处理...")
        print(f"📂 输入文件: {input_file}")
        
        debug_dir = target_dirs[0]
        binary_output = debug_dir / "site_cn_binary.dat"
        
        print(f"📂 输出文件将保存到: {debug_dir}")
        
        extract_and_split_domains(str(input_file), str(binary_output))
        
        release_dir = target_dirs[1]
        if release_dir.exists():
            print(f"\n📦 正在复制文件到 {release_dir}")
            import shutil
            
            shutil.copy2(binary_output, release_dir / binary_output.name)
            print(f"  ✓ 已复制 {binary_output.name}")
            
            txt_output = binary_output.with_suffix('.txt')
            shutil.copy2(txt_output, release_dir / txt_output.name)
            print(f"  ✓ 已复制 {txt_output.name}")
        
        print("\n✨ 处理完成！")
        
    except Exception as e:
        print(f"\n❌ 错误: {e}")
        sys.exit(1)

if __name__ == "__main__":
    main()