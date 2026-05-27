pub fn compute_stride_by_base(rdx: i32) -> i32 {
    // 返回将一个“位”用 16 进制字符串表达所需的最小长度（rdx<=16 时为 1，rdx<=16^2 时为 2，依此类推）
    const BASE_: i32 = 16;
    if rdx <= 0 {
        return 0;
    }
    let mut power = 1;
    let mut rdx_ = rdx;
    while rdx_ > BASE_ {
        power += 1;
        rdx_ /= BASE_;
    }
    power
}

pub fn int_array_to_string(int_array: &[i32], rdx: i32) -> String {
    let power = compute_stride_by_base(rdx);
    let mut ret = String::new();

    for &val in int_array {
        let cur = int_to_hex_string(val);
        let pad_len = power as isize - cur.len() as isize;
        if pad_len > 0 {
            ret.push_str(&"0".repeat(pad_len as usize));
        }
        ret.push_str(&cur);
    }

    ret
}

pub fn int_to_hex_string(num: i32) -> String {
    if num < 0 {
        return String::new();
    }
    if num < 10 {
        return num.to_string();
    } else if num < 16 {
        return String::from((b'a' + (num - 10) as u8) as char).to_string();
    }
    let cur = num % 16;
    if cur > 9 {
        return int_to_hex_string(num / 16) + &String::from((b'a' + (cur - 10) as u8) as char);
    } else {
        return int_to_hex_string(num / 16) + &cur.to_string();
    }
}

// --------
// 路由位生成（带深度扰动）
// 为避免同一把 key 在不同深度上得到相同的位（导致分裂后仍聚集一个子桶），
// 我们采用“key 哈希 + 深度扰动”的方式生成第 pos 位的数字。

fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for &b in bytes {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

fn splitmix64(mut x: u64) -> u64 {
    x = x.wrapping_add(0x9e3779b97f4a7c15);
    let mut z = x;
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d049bb133111eb);
    z ^ (z >> 31)
}

pub fn digit_at_mixed(key: &str, rdx: i32, pos: usize) -> i32 {
    if rdx <= 0 {
        return 0;
    }
    let h_key = fnv1a64(key.as_bytes());
    // 将 pos 与 key 哈希混合，生成稳定且与深度相关的位
    let h = splitmix64(h_key ^ (pos as u64));
    (h % (rdx as u64)) as i32
}

pub fn digits_prefix_mixed(key: &str, rdx: i32, len: usize) -> Vec<i32> {
    (0..len).map(|i| digit_at_mixed(key, rdx, i)).collect()
}
