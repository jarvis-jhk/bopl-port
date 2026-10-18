//! The .NET introspective sort (engine.md §4.1). Not stable; reproduced
//! exactly so that ties resolve like the original.

use std::cmp::Ordering;

/// Sorts `x` in place with the runtime's `IntrospectiveSort`.
pub fn introsort<T: Copy, F: FnMut(&T, &T) -> Ordering>(x: &mut [T], mut cmp: F) {
    let n = x.len();
    if n < 2 {
        return;
    }
    let mut l = 0u32;
    let mut m = n;
    while m > 0 {
        l += 1;
        m /= 2;
    }
    intro(x, 0, n as isize - 1, 2 * l as i32, &mut cmp);
}

fn swap_if_greater<T: Copy, F: FnMut(&T, &T) -> Ordering>(
    x: &mut [T],
    a: isize,
    b: isize,
    cmp: &mut F,
) {
    if a != b && cmp(&x[a as usize], &x[b as usize]) == Ordering::Greater {
        x.swap(a as usize, b as usize);
    }
}

fn intro<T: Copy, F: FnMut(&T, &T) -> Ordering>(
    x: &mut [T],
    lo: isize,
    mut hi: isize,
    mut depth: i32,
    cmp: &mut F,
) {
    while hi > lo {
        let size = hi - lo + 1;
        if size <= 16 {
            if size == 2 {
                swap_if_greater(x, lo, hi, cmp);
            } else if size == 3 {
                swap_if_greater(x, lo, hi - 1, cmp);
                swap_if_greater(x, lo, hi, cmp);
                swap_if_greater(x, hi - 1, hi, cmp);
            } else {
                insertion(x, lo, hi, cmp);
            }
            return;
        }
        if depth == 0 {
            heapsort(x, lo, hi, cmp);
            return;
        }
        depth -= 1;
        let p = partition(x, lo, hi, cmp);
        intro(x, p + 1, hi, depth, cmp);
        hi = p - 1;
    }
}

fn partition<T: Copy, F: FnMut(&T, &T) -> Ordering>(
    x: &mut [T],
    lo: isize,
    hi: isize,
    cmp: &mut F,
) -> isize {
    let mid = lo + (hi - lo) / 2;
    swap_if_greater(x, lo, mid, cmp);
    swap_if_greater(x, lo, hi, cmp);
    swap_if_greater(x, mid, hi, cmp);
    let pivot = x[mid as usize];
    x.swap(mid as usize, (hi - 1) as usize);
    let mut l = lo;
    let mut r = hi - 1;
    while l < r {
        loop {
            l += 1;
            if cmp(&x[l as usize], &pivot) != Ordering::Less {
                break;
            }
        }
        loop {
            r -= 1;
            if cmp(&pivot, &x[r as usize]) != Ordering::Less {
                break;
            }
        }
        if l >= r {
            break;
        }
        x.swap(l as usize, r as usize);
    }
    if l != hi - 1 {
        x.swap(l as usize, (hi - 1) as usize);
    }
    l
}

fn insertion<T: Copy, F: FnMut(&T, &T) -> Ordering>(
    x: &mut [T],
    lo: isize,
    hi: isize,
    cmp: &mut F,
) {
    let mut i = lo;
    while i < hi {
        let t = x[(i + 1) as usize];
        let mut j = i;
        while j >= lo && cmp(&t, &x[j as usize]) == Ordering::Less {
            x[(j + 1) as usize] = x[j as usize];
            j -= 1;
        }
        x[(j + 1) as usize] = t;
        i += 1;
    }
}

fn heapsort<T: Copy, F: FnMut(&T, &T) -> Ordering>(x: &mut [T], lo: isize, hi: isize, cmp: &mut F) {
    let n = hi - lo + 1;
    let mut i = n / 2;
    while i >= 1 {
        down_heap(x, i, n, lo, cmp);
        i -= 1;
    }
    let mut i = n;
    while i > 1 {
        x.swap(lo as usize, (lo + i - 1) as usize);
        down_heap(x, 1, i - 1, lo, cmp);
        i -= 1;
    }
}

fn down_heap<T: Copy, F: FnMut(&T, &T) -> Ordering>(
    x: &mut [T],
    mut i: isize,
    n: isize,
    lo: isize,
    cmp: &mut F,
) {
    let d = x[(lo + i - 1) as usize];
    while i <= n / 2 {
        let mut c = 2 * i;
        if c < n && cmp(&x[(lo + c - 1) as usize], &x[(lo + c) as usize]) == Ordering::Less {
            c += 1;
        }
        if cmp(&d, &x[(lo + c - 1) as usize]) != Ordering::Less {
            break;
        }
        x[(lo + i - 1) as usize] = x[(lo + c - 1) as usize];
        i = c;
    }
    x[(lo + i - 1) as usize] = d;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sorts_like_a_sort() {
        let mut s = 12345u32;
        for n in [0usize, 1, 2, 3, 5, 16, 17, 40, 200, 1000] {
            let mut v: Vec<i32> = (0..n)
                .map(|_| {
                    s ^= s << 13;
                    s ^= s >> 17;
                    s ^= s << 5;
                    (s % 50) as i32
                })
                .collect();
            let mut w = v.clone();
            introsort(&mut v, |a, b| a.cmp(b));
            w.sort();
            assert_eq!(v, w);
        }
    }
}
