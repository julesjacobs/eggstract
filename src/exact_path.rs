use crate::graph::{FastGraph, NONE};
use std::cmp::Reverse;
use std::collections::BinaryHeap;
use std::time::{Duration, Instant};

// Binary scaling preserves the exact real value of each input f64. The sum of
// all node weights must fit, so every subset sum below is also safe.
pub(super) fn integer_weights(costs: &[f64]) -> Option<Vec<u128>> {
    let parts: Vec<_> = costs
        .iter()
        .map(|&x| {
            if !x.is_finite() || x < 0.0 {
                return None;
            }
            if x == 0.0 {
                return Some((0u64, 0i32));
            }
            let b = x.to_bits();
            let e = ((b >> 52) & 2047) as i32;
            let m = (b & ((1u64 << 52) - 1)) | if e == 0 { 0 } else { 1u64 << 52 };
            let z = m.trailing_zeros();
            Some((m >> z, if e == 0 { -1074 } else { e - 1075 } + z as i32))
        })
        .collect::<Option<_>>()?;
    let exponent = parts
        .iter()
        .filter(|p| p.0 != 0)
        .map(|p| p.1)
        .min()
        .unwrap_or(0);
    let mut sum = 0u128;
    parts
        .iter()
        .map(|&(m, e)| {
            let w = if m == 0 {
                0
            } else {
                let shift = (e - exponent) as u32;
                if shift >= 128 || (m as u128) > (u128::MAX >> shift) {
                    return None;
                }
                (m as u128) << shift
            };
            sum = sum.checked_add(w)?;
            Some(w)
        })
        .collect()
}

pub(super) struct PathBound {
    weights: Vec<u128>,
    lower: u128,
    unit: f64,
}

impl PathBound {
    pub fn new(g: &FastGraph<'_>, start: Instant, deadline: Duration) -> Option<Self> {
        let weights = integer_weights(&g.costs)?;
        let unit = weights
            .iter()
            .position(|&w| w != 0)
            .map_or(1.0, |i| g.costs[i] / weights[i] as f64);
        let mut pending: Vec<_> = g.child_offsets.windows(2).map(|w| w[1] - w[0]).collect();
        let mut maxima = vec![0u128; weights.len()];
        let mut distances = vec![None; g.class_ids.len()];
        let mut root = vec![false; distances.len()];
        let mut roots_left = 0;
        for &c in &g.roots {
            if !root[c] {
                root[c] = true;
                roots_left += 1;
            }
        }
        let mut heap = BinaryHeap::new();
        for (n, &count) in pending.iter().enumerate() {
            if count == 0 {
                heap.push(Reverse((weights[n], n)));
            }
        }
        let mut work = 0;
        while let Some(Reverse((value, n))) = heap.pop() {
            work += 1;
            if work & 255 == 1 && start.elapsed() >= deadline {
                return None;
            }
            let c = g.node_class[n];
            if distances[c].is_some() {
                continue;
            }
            distances[c] = Some(value);
            if root[c] {
                roots_left -= 1;
                if roots_left == 0 {
                    let lower = g
                        .roots
                        .iter()
                        .map(|&r| distances[r].unwrap())
                        .max()
                        .unwrap();
                    return Some(Self {
                        weights,
                        lower,
                        unit,
                    });
                }
            }
            for &p in g.parents(c) {
                if distances[g.node_class[p]].is_some() {
                    continue;
                }
                pending[p] -= 1;
                maxima[p] = maxima[p].max(value);
                if pending[p] == 0 {
                    heap.push(Reverse((weights[p].checked_add(maxima[p])?, p)));
                }
            }
        }
        None
    }
    pub fn matches(&self, choices: &[usize]) -> bool {
        choices
            .iter()
            .filter(|&&n| n != NONE)
            .map(|&n| self.weights[n])
            .sum::<u128>()
            == self.lower
    }
    pub fn as_float(&self) -> f64 {
        // Both rounding steps preserve a lower bound; unit is a binary power.
        ((self.lower as f64).next_down().max(0.0) * self.unit)
            .next_down()
            .max(0.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{verify, ClassId, EGraph, NodeId};
    use egraph_serialize::{Cost, Node};

    #[test]
    fn exact_path_is_below_every_valid_rooted_dag() {
        let mut random = 817u64;
        let mut next = || {
            random ^= random << 13;
            random ^= random >> 7;
            random ^= random << 17;
            random
        };
        let mut checked = 0;
        for _ in 0..100 {
            let mut input = EGraph::default();
            for c in 0..4 {
                for a in 0..3 {
                    let children = (0..if a == 0 { 0 } else { next() % 3 })
                        .map(|_| NodeId::from(format!("{}-0", next() % 4)))
                        .collect();
                    input.add_node(
                        format!("{c}-{a}"),
                        Node {
                            op: "f".into(),
                            eclass: ClassId::from(c.to_string()),
                            cost: Cost::new((next() % 9) as f64 / 8.0).unwrap(),
                            children,
                            subsumed: false,
                        },
                    );
                }
            }
            let roots = vec![ClassId::from("0"), ClassId::from("1")];
            let start = Instant::now();
            let graph = FastGraph::new(&input, &roots).unwrap();
            let bound = PathBound::new(&graph, start, Duration::from_secs(60)).unwrap();
            for mut assignment in 0..81 {
                let choices: Vec<_> = (0..4)
                    .map(|c| {
                        let n = c * 3 + assignment % 3;
                        assignment /= 3;
                        n
                    })
                    .collect();
                let mut reached = vec![NONE; 4];
                let mut stack = graph.roots.clone();
                while let Some(c) = stack.pop() {
                    if reached[c] != NONE {
                        continue;
                    }
                    reached[c] = choices[c];
                    stack.extend(graph.children(choices[c]));
                }
                let selected: Vec<_> = reached
                    .iter()
                    .filter(|&&n| n != NONE)
                    .map(|&n| input.nodes.get_index(n).unwrap().0.clone())
                    .collect();
                if verify(&input, &roots, &selected).is_err() {
                    continue;
                }
                checked += 1;
                let cost: u128 = reached
                    .iter()
                    .filter(|&&n| n != NONE)
                    .map(|&n| bound.weights[n])
                    .sum();
                assert!(bound.lower <= cost);
                assert_eq!(bound.matches(&reached), bound.lower == cost);
            }
        }
        assert!(checked > 1000);
    }

    #[test]
    fn exact_weight_conversion_rejects_unrepresentable_spans_and_sums() {
        assert_eq!(integer_weights(&[0.0, 0.125, 0.5]), Some(vec![0, 1, 4]));
        assert!(integer_weights(&[f64::from_bits(1), f64::MAX]).is_none());
        assert!(integer_weights(&[f64::INFINITY]).is_none());
        assert!(integer_weights(&[-1.0]).is_none());
    }
}
