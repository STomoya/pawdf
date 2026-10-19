//! Resolving overlapping code or CID ranges into disjoint ones.

use std::collections::BTreeMap;

/// Disjoint `(lo, hi, value)` ranges sorted by `lo`, where each point takes the value of
/// the last input range covering it. `shift(v, d)` is the value of a piece starting `d`
/// points after the start of the range that had value `v`. Ranges with `lo > hi` are
/// skipped.
pub(crate) fn last_wins<T: Copy>(
    ranges: impl IntoIterator<Item = (u32, u32, T)>,
    shift: impl Fn(T, u32) -> T,
) -> Vec<(u32, u32, T)> {
    let mut map: BTreeMap<u32, (u32, T)> = BTreeMap::new();
    for (lo, hi, v) in ranges {
        if lo > hi {
            continue;
        }
        if let Some((&s, &(e, w))) = map.range(..lo).next_back()
            && e >= lo
        {
            map.insert(s, (lo - 1, w));
            if e > hi {
                map.insert(hi + 1, (e, shift(w, hi + 1 - s)));
            }
        }
        if let Some((&s, &(e, w))) = map.range(lo..=hi).next_back()
            && e > hi
        {
            map.insert(hi + 1, (e, shift(w, hi + 1 - s)));
        }
        while let Some((&k, _)) = map.range(lo..=hi).next() {
            map.remove(&k);
        }
        map.insert(lo, (hi, v));
    }
    map.into_iter().map(|(lo, (hi, v))| (lo, hi, v)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn add(v: u32, d: u32) -> u32 {
        v + d
    }

    #[test]
    fn overlap_later_range_wins() {
        assert_eq!(
            last_wins([(0, 20, 0), (10, 30, 1000)], add),
            [(0, 9, 0), (10, 30, 1000)]
        );
        assert_eq!(
            last_wins([(10, 30, 100), (0, 20, 0)], add),
            [(0, 20, 0), (21, 30, 111)]
        );
        assert_eq!(
            last_wins([(0, 30, 0), (10, 12, 500)], add),
            [(0, 9, 0), (10, 12, 500), (13, 30, 13)]
        );
        assert_eq!(last_wins([(5, 5, 1), (5, 5, 2)], add), [(5, 5, 2)]);
        assert_eq!(last_wins([(5, 4, 1)], add), []);
        assert_eq!(
            last_wins([(0, u32::MAX, 0), (u32::MAX, u32::MAX, 7)], add),
            [(0, u32::MAX - 1, 0), (u32::MAX, u32::MAX, 7)]
        );
    }

    #[test]
    fn many_overlaps_resolve_quickly() {
        let start = std::time::Instant::now();
        let out = last_wins((0..50_000).map(|i| (i, 60_000 - i, i)), |v, _| v);
        assert!(start.elapsed().as_secs_f64() < 1.);
        let at = |p: u32| out.iter().find(|r| r.0 <= p && p <= r.1).map(|r| r.2);
        // Ranges i ≤ 30 000 ≤ 60 000 - i cover it; the last is i = 30 000.
        assert_eq!(at(30_000), Some(30_000));
        assert_eq!(at(0), Some(0));
        assert!(out.windows(2).all(|w| w[0].1 < w[1].0));
    }
}
