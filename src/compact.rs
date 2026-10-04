use crate::graph::{FastGraph, NONE};
use crate::{Error, Options};
use egraph_serialize::Cost;
use rustc_hash::FxHashMap;
use std::cmp::Reverse;
use std::collections::BinaryHeap;
use std::time::{Duration, Instant};

#[derive(Clone)]
struct Active {
    choices: Vec<usize>,
    refs: Vec<usize>,
    cost: f64,
}

impl Active {
    fn seed_with_path(graph: &FastGraph<'_>, choices: &[usize]) -> Option<(Self, bool)> {
        let mut active = vec![NONE; choices.len()];
        let mut refs = vec![0; choices.len()];
        // 0 is unseen, 1 is visiting; a finished value is positive-path length + 2.
        let mut order = vec![0usize; choices.len()];
        let mut stack = Vec::new();
        let mut cost = 0.0;
        let mut positive = 0;
        for &root in &graph.roots {
            refs[root] += 1;
            stack.push((root, false));
            while let Some((c, leaving)) = stack.pop() {
                if leaving {
                    order[c] = 2
                        + usize::from(graph.costs[active[c]] > 0.0)
                        + graph
                            .children(active[c])
                            .iter()
                            .map(|&d| order[d] - 2)
                            .max()
                            .unwrap_or(0);
                    continue;
                }
                if order[c] >= 2 {
                    continue;
                }
                if order[c] == 1 || choices[c] == NONE {
                    return None;
                }
                let n = choices[c];
                if graph.node_class[n] != c {
                    return None;
                }
                order[c] = 1;
                active[c] = n;
                cost += graph.costs[n];
                positive += usize::from(graph.costs[n] > 0.0);
                stack.push((c, true));
                for &d in graph.children(n).iter().rev() {
                    refs[d] += 1;
                    stack.push((d, false));
                }
            }
        }
        let admitted = graph.roots.iter().map(|&c| order[c] - 2).max().unwrap_or(0) == positive;
        Some((
            Self {
                choices: active,
                refs,
                cost,
            },
            admitted,
        ))
    }

    fn new(graph: &FastGraph<'_>, choices: &[usize]) -> Option<Self> {
        let mut active = vec![NONE; choices.len()];
        let mut refs = vec![0usize; choices.len()];
        let mut color = vec![0u8; choices.len()];
        let mut stack = Vec::new();
        let mut cost = 0.0;
        for &root in &graph.roots {
            refs[root] += 1;
            stack.push((root, false));
            while let Some((c, leaving)) = stack.pop() {
                if leaving {
                    color[c] = 2;
                    continue;
                }
                if color[c] == 2 {
                    continue;
                }
                if color[c] == 1 || choices[c] == NONE {
                    return None;
                }
                let n = choices[c];
                if graph.node_class[n] != c {
                    return None;
                }
                color[c] = 1;
                active[c] = n;
                cost += graph.costs[n];
                stack.push((c, true));
                for &d in graph.children(n).iter().rev() {
                    refs[d] += 1;
                    stack.push((d, false));
                }
            }
        }
        Some(Self {
            choices: active,
            refs,
            cost,
        })
    }
}

struct Scratch {
    poll_ticks: usize,
    eligible: Option<Vec<bool>>,
    components: Option<Vec<usize>>,
    marks: Vec<u32>,
    counts: Vec<usize>,
    touched: Vec<usize>,
    added: Vec<usize>,
    removed: Vec<usize>,
    stack: Vec<usize>,
    seen: Vec<u32>,
    epoch: u32,
    trials: usize,
    moves: usize,
}

impl Scratch {
    fn budget_expired(&mut self, start: Instant, deadline: Duration) -> bool {
        self.poll_ticks = self.poll_ticks.wrapping_add(1);
        (self.poll_ticks & 63 == 0) && start.elapsed() >= deadline
    }

    fn new(classes: usize) -> Self {
        Self {
            poll_ticks: 63,
            eligible: None,
            components: None,
            marks: vec![0; classes],
            counts: vec![0; classes],
            touched: Vec::new(),
            added: Vec::new(),
            removed: Vec::new(),
            stack: Vec::new(),
            seen: vec![0; classes],
            epoch: 0,
            trials: 0,
            moves: 0,
        }
    }

    fn touch(&mut self, state: &Active, c: usize) -> usize {
        if self.marks[c] != self.epoch {
            self.marks[c] = self.epoch;
            self.counts[c] = state.refs[c];
            self.touched.push(c);
        }
        self.counts[c]
    }

    fn improve(
        &mut self,
        graph: &FastGraph<'_>,
        state: &mut Active,
        fallback: &[usize],
        n: usize,
        start: Instant,
        deadline: Duration,
    ) -> bool {
        let Some(delta) = self.evaluate(
            graph,
            state,
            fallback,
            n,
            start,
            deadline,
            -f64::from_bits(1),
        ) else {
            return false;
        };
        if delta >= 0.0 {
            return false;
        }
        self.apply(state, fallback, graph.node_class[n], n, delta);
        true
    }

    #[allow(clippy::too_many_arguments)]
    fn evaluate(
        &mut self,
        graph: &FastGraph<'_>,
        state: &Active,
        fallback: &[usize],
        n: usize,
        start: Instant,
        deadline: Duration,
        max_delta: f64,
    ) -> Option<f64> {
        self.evaluate_avoiding(graph, state, fallback, n, start, deadline, max_delta, None)
    }

    #[allow(clippy::too_many_arguments)]
    fn evaluate_avoiding(
        &mut self,
        graph: &FastGraph<'_>,
        state: &Active,
        fallback: &[usize],
        n: usize,
        start: Instant,
        deadline: Duration,
        max_delta: f64,
        forbidden: Option<usize>,
    ) -> Option<f64> {
        self.evaluate_trial(
            graph, state, fallback, n, start, deadline, max_delta, forbidden, true,
        )
    }

    fn price(
        &mut self,
        graph: &FastGraph<'_>,
        state: &Active,
        fallback: &[usize],
        n: usize,
        start: Instant,
        deadline: Duration,
    ) -> Option<f64> {
        self.evaluate_trial(
            graph,
            state,
            fallback,
            n,
            start,
            deadline,
            f64::MAX,
            None,
            false,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn evaluate_trial(
        &mut self,
        graph: &FastGraph<'_>,
        state: &Active,
        fallback: &[usize],
        n: usize,
        start: Instant,
        deadline: Duration,
        max_delta: f64,
        forbidden: Option<usize>,
        validate: bool,
    ) -> Option<f64> {
        if self.eligible.as_ref().is_some_and(|eligible| !eligible[n]) {
            return None;
        }
        let c = graph.node_class[n];
        let old = state.choices[c];
        if old == NONE || old == n {
            return None;
        }
        self.trials += 1;
        if self.trials & 255 == 0 && start.elapsed() >= deadline {
            return None;
        }
        self.epoch = self.epoch.wrapping_add(1);
        if self.epoch == 0 {
            self.marks.fill(0);
            self.seen.fill(0);
            self.epoch = 1;
        }
        self.touched.clear();
        self.added.clear();
        self.removed.clear();
        self.stack.clear();
        self.stack.extend(graph.children(n));
        let mut delta = graph.costs[n] - graph.costs[old];
        let mut work = 0usize;
        while let Some(d) = self.stack.pop() {
            work += 1;
            if work & 255 == 0 && start.elapsed() >= deadline {
                return None;
            }
            let previous = self.touch(state, d);
            self.counts[d] += 1;
            if previous == 0 {
                let node = fallback[d];
                if node == NONE {
                    return None;
                }
                if self.added.len() >= 512 {
                    return None;
                }
                self.added.push(d);
                delta += graph.costs[node];
                self.stack.extend(graph.children(node));
            }
        }
        self.stack.extend(graph.children(old));
        while let Some(d) = self.stack.pop() {
            work += 1;
            if work & 255 == 0 && start.elapsed() >= deadline {
                return None;
            }
            let previous = self.touch(state, d);
            if previous == 0 {
                return None;
            }
            self.counts[d] -= 1;
            if previous == 1 {
                let node = state.choices[d];
                if node == NONE {
                    return None;
                }
                if self.removed.len() >= 4096 {
                    return None;
                }
                self.removed.push(d);
                delta -= graph.costs[node];
                self.stack.extend(graph.children(node));
            }
        }
        if !delta.is_finite() {
            return None;
        }
        if delta > max_delta {
            return None;
        }
        if !validate {
            return Some(delta);
        }
        self.stack.extend(graph.children(n));
        while let Some(d) = self.stack.pop() {
            if d == c || forbidden == Some(d) {
                return None;
            }
            if forbidden.is_none() && self.components.as_ref().is_some_and(|ids| ids[d] != ids[c]) {
                continue;
            }
            if self.seen[d] == self.epoch {
                continue;
            }
            self.seen[d] = self.epoch;
            work += 1;
            if work & 255 == 0 && start.elapsed() >= deadline {
                return None;
            }
            let node = if state.choices[d] != NONE {
                state.choices[d]
            } else {
                fallback[d]
            };
            if node == NONE {
                return None;
            }
            self.stack.extend(graph.children(node));
        }
        Some(delta)
    }

    fn apply(&mut self, state: &mut Active, fallback: &[usize], c: usize, n: usize, delta: f64) {
        state.choices[c] = n;
        for &d in &self.added {
            state.choices[d] = fallback[d];
        }
        for &d in &self.removed {
            state.choices[d] = NONE;
        }
        for &d in &self.touched {
            state.refs[d] = self.counts[d];
        }
        state.cost += delta;
        self.moves += 1;
    }
}

fn path_lower_bound(graph: &FastGraph<'_>, start: Instant, deadline: Duration) -> Option<f64> {
    let mut pending: Vec<_> = graph
        .child_offsets
        .windows(2)
        .map(|w| w[1] - w[0])
        .collect();
    let mut maxima = vec![0.0f64; graph.costs.len()];
    let mut distances = vec![None; graph.class_ids.len()];
    let mut heap = BinaryHeap::new();
    for (n, &count) in pending.iter().enumerate() {
        if n & 255 == 0 && start.elapsed() >= deadline {
            return None;
        }
        if count == 0 {
            heap.push(Reverse((Cost::new(graph.costs[n]).unwrap(), n)));
        }
    }
    let mut work = 0usize;
    while let Some(Reverse((cost, n))) = heap.pop() {
        work += 1;
        if work & 255 == 0 && start.elapsed() >= deadline {
            return None;
        }
        let c = graph.node_class[n];
        if distances[c].is_some() {
            continue;
        }
        let value = cost.into_inner();
        distances[c] = Some(value);
        if graph.roots.iter().all(|&r| distances[r].is_some()) {
            return Some(
                graph
                    .roots
                    .iter()
                    .map(|&r| distances[r].unwrap())
                    .fold(0.0, f64::max),
            );
        }
        for &parent in graph.parents(c) {
            pending[parent] -= 1;
            maxima[parent] = maxima[parent].max(value);
            if pending[parent] == 0 {
                let bound = if graph.costs[parent] == 0.0 {
                    maxima[parent]
                } else {
                    (graph.costs[parent] + maxima[parent])
                        .next_down()
                        .max(maxima[parent])
                        .max(graph.costs[parent])
                };
                if bound.is_finite() {
                    heap.push(Reverse((Cost::new(bound).unwrap(), parent)));
                }
            }
        }
    }
    None
}

fn meets_path_bound(graph: &FastGraph<'_>, state: &Active, lower: Option<f64>) -> bool {
    let Some(lower) = lower else {
        return false;
    };
    let upper = state
        .choices
        .iter()
        .filter(|&&n| n != NONE)
        .fold(0.0f64, |total, &n| {
            let cost = graph.costs[n];
            if cost == 0.0 {
                total
            } else if total == 0.0 {
                cost
            } else {
                (total + cost).next_up()
            }
        });
    upper.is_finite() && upper - lower <= upper * 1e-12
}

fn prune_equivalent(
    graph: &FastGraph<'_>,
    options: &mut [Vec<usize>],
    start: Instant,
    deadline: Duration,
) -> Vec<bool> {
    let mut eligible = vec![true; graph.costs.len()];
    for (c, alternatives) in options.iter_mut().enumerate() {
        if c & 255 == 0 && start.elapsed() >= deadline {
            break;
        }
        let mut representatives: FxHashMap<Vec<usize>, usize> = FxHashMap::default();
        for &n in alternatives.iter() {
            let mut children = graph.children(n).to_vec();
            children.sort_unstable();
            children.dedup();
            match representatives.entry(children) {
                std::collections::hash_map::Entry::Vacant(entry) => {
                    entry.insert(n);
                }
                std::collections::hash_map::Entry::Occupied(mut entry) => {
                    let previous = *entry.get();
                    if graph.costs[n] < graph.costs[previous] {
                        eligible[previous] = false;
                        entry.insert(n);
                    } else {
                        eligible[n] = false;
                    }
                }
            }
        }
        alternatives.retain(|&n| eligible[n]);
    }
    eligible
}

fn rebuild(
    graph: &FastGraph<'_>,
    demand: &[f64],
    strength: f64,
    preferred: &[usize],
    start: Instant,
    deadline: Duration,
) -> Option<Vec<usize>> {
    let factors: Vec<_> = demand.iter().map(|d| d.max(1.0).powf(-strength)).collect();
    let mut pending: Vec<_> = (0..graph.costs.len())
        .map(|n| graph.children(n).len())
        .collect();
    let mut costs: Vec<_> = graph
        .costs
        .iter()
        .enumerate()
        .map(|(n, &cost)| cost * factors[graph.node_class[n]])
        .collect();
    let mut heap = BinaryHeap::new();
    let mut queued_best = vec![None; demand.len()];
    for (n, &p) in pending.iter().enumerate() {
        if n & 1023 == 0 && start.elapsed() >= deadline {
            return None;
        }
        if p == 0 {
            let c = graph.node_class[n];
            let key = (Cost::new(costs[n]).unwrap(), preferred[c] != n, n);
            if queued_best[c].is_none_or(|old| key < old) {
                queued_best[c] = Some(key);
                heap.push(Reverse(key));
            }
        }
    }
    let mut chosen = vec![NONE; demand.len()];
    let mut work = 0usize;
    while let Some(Reverse((cost, _, n))) = heap.pop() {
        work += 1;
        if work & 1023 == 0 && start.elapsed() >= deadline {
            return None;
        }
        let c = graph.node_class[n];
        if chosen[c] != NONE {
            continue;
        }
        chosen[c] = n;
        for &parent in graph.parents(c) {
            work += 1;
            if work & 1023 == 0 && start.elapsed() >= deadline {
                return None;
            }
            if chosen[graph.node_class[parent]] != NONE {
                continue;
            }
            costs[parent] += cost.into_inner();
            pending[parent] -= 1;
            if pending[parent] == 0 {
                let c = graph.node_class[parent];
                let key = (
                    Cost::new(costs[parent]).unwrap(),
                    preferred[c] != parent,
                    parent,
                );
                if queued_best[c].is_none_or(|old| key < old) {
                    queued_best[c] = Some(key);
                    heap.push(Reverse(key));
                }
            }
        }
    }
    Some(chosen)
}

fn dependency_components(
    graph: &FastGraph<'_>,
    options: &[Vec<usize>],
    start: Instant,
    deadline: Duration,
) -> Option<Vec<usize>> {
    let mut seen = vec![false; options.len()];
    let mut order = Vec::with_capacity(options.len());
    let mut stack = Vec::new();
    let mut work = 0usize;
    for root in 0..options.len() {
        if seen[root] {
            continue;
        }
        stack.push((root, false));
        while let Some((c, leaving)) = stack.pop() {
            work += 1;
            if work & 255 == 0 && start.elapsed() >= deadline {
                return None;
            }
            if leaving {
                order.push(c);
                continue;
            }
            if seen[c] {
                continue;
            }
            seen[c] = true;
            stack.push((c, true));
            for &n in &options[c] {
                for &d in graph.children(n) {
                    if !seen[d] {
                        stack.push((d, false));
                    }
                }
            }
        }
    }
    let mut ids = vec![NONE; options.len()];
    let mut pending = Vec::new();
    for root in order.into_iter().rev() {
        if ids[root] != NONE {
            continue;
        }
        pending.push(root);
        ids[root] = root;
        while let Some(c) = pending.pop() {
            for &n in graph.parents(c) {
                work += 1;
                if work & 255 == 0 && start.elapsed() >= deadline {
                    return None;
                }
                let d = graph.node_class[n];
                if ids[d] == NONE {
                    ids[d] = root;
                    pending.push(d);
                }
            }
        }
    }
    Some(ids)
}

fn mandatory_classes(
    graph: &FastGraph<'_>,
    options: &[Vec<usize>],
    start: Instant,
    deadline: Duration,
) -> Vec<bool> {
    let mut mandatory = vec![false; options.len()];
    let mut queue = Vec::new();
    for &r in &graph.roots {
        if !mandatory[r] {
            mandatory[r] = true;
            queue.push(r);
        }
    }
    let mut work = 0usize;
    while let Some(c) = queue.pop() {
        let Some(&first) = options[c].first() else {
            continue;
        };
        for &d in graph.children(first) {
            if mandatory[d] {
                continue;
            }
            let mut common = true;
            for &n in &options[c] {
                work += 1;
                if work & 255 == 0 && start.elapsed() >= deadline {
                    return mandatory;
                }
                if !graph.children(n).contains(&d) {
                    common = false;
                    break;
                }
            }
            if common {
                mandatory[d] = true;
                queue.push(d);
            }
        }
    }
    mandatory
}

#[allow(clippy::too_many_arguments)]
fn retire_support(
    graph: &FastGraph<'_>,
    state: &mut Active,
    fallback: &[usize],
    options: &[Vec<usize>],
    scratch: &mut Scratch,
    target: usize,
    start: Instant,
    deadline: Duration,
) -> bool {
    if graph.roots.contains(&target) || state.choices[target] == NONE {
        return false;
    }
    let original_cost = state.cost;
    let mut undo = Vec::new();
    let mut consumers: Vec<_> = graph
        .parents(target)
        .iter()
        .copied()
        .filter(|&n| state.choices[graph.node_class[n]] == n)
        .map(|n| graph.node_class[n])
        .collect();
    consumers.sort_unstable();
    consumers.dedup();
    let mut proposals = 0usize;
    let mut ranked = Vec::new();
    for c in consumers {
        for &n in &options[c] {
            proposals += 1;
            if proposals & 255 == 0 && start.elapsed() >= deadline {
                break;
            }
            if graph.children(n).contains(&target) {
                continue;
            }
            if let Some(delta) = scratch.price(graph, state, fallback, n, start, deadline) {
                ranked.push((delta, n));
            }
        }
        if scratch.budget_expired(start, deadline) {
            break;
        }
    }
    ranked.sort_by(|a, b| a.0.total_cmp(&b.0));
    for (_, n) in ranked {
        if state.refs[target] == 0 || scratch.budget_expired(start, deadline) {
            break;
        }
        let old = state.choices[graph.node_class[n]];
        if old == NONE || !graph.children(old).contains(&target) {
            continue;
        }
        let Some(delta) = scratch.evaluate_avoiding(
            graph,
            state,
            fallback,
            n,
            start,
            deadline,
            f64::MAX,
            Some(target),
        ) else {
            continue;
        };
        let c = graph.node_class[n];
        undo.push((c, state.choices[c], state.refs[c]));
        for &d in &scratch.touched {
            undo.push((d, state.choices[d], state.refs[d]));
        }
        scratch.apply(state, fallback, c, n, delta);
    }
    if state.refs[target] == 0 && state.cost < original_cost {
        if let Some(checked) = Active::new(graph, &state.choices) {
            if checked.cost < original_cost {
                *state = checked;
                return true;
            }
        }
    }
    for (c, choice, refs) in undo.into_iter().rev() {
        state.choices[c] = choice;
        state.refs[c] = refs;
    }
    state.cost = original_cost;
    false
}

fn open_support(
    graph: &FastGraph<'_>,
    state: &mut Active,
    fallback: &[usize],
    scratch: &mut Scratch,
    target: usize,
    start: Instant,
    deadline: Duration,
) -> bool {
    let original_cost = state.cost;
    let mut undo = Vec::new();
    let mut locked = Vec::new();
    let mut best_cost = original_cost;
    let mut best_checkpoint = 0;
    let mut ranked = Vec::new();
    for (i, &n) in graph.parents(target).iter().enumerate() {
        if i & 255 == 0 && start.elapsed() >= deadline {
            break;
        }
        let old = state.choices[graph.node_class[n]];
        if old == NONE || graph.children(old).contains(&target) {
            continue;
        }
        if let Some(delta) = scratch.price(graph, state, fallback, n, start, deadline) {
            ranked.push((delta, n));
        }
    }
    ranked.sort_by(|a, b| a.0.total_cmp(&b.0));
    for (_, n) in ranked {
        if scratch.budget_expired(start, deadline) {
            break;
        }
        let c = graph.node_class[n];
        let old = state.choices[c];
        if old == NONE || locked.contains(&c) || graph.children(old).contains(&target) {
            continue;
        }
        let Some(delta) = scratch.evaluate(graph, state, fallback, n, start, deadline, f64::MAX)
        else {
            continue;
        };
        let c = graph.node_class[n];
        locked.push(c);
        undo.push((c, state.choices[c], state.refs[c]));
        for &d in &scratch.touched {
            undo.push((d, state.choices[d], state.refs[d]));
        }
        scratch.apply(state, fallback, c, n, delta);
        if state.cost < best_cost {
            best_cost = state.cost;
            best_checkpoint = undo.len();
        }
    }
    for &(c, choice, refs) in undo[best_checkpoint..].iter().rev() {
        state.choices[c] = choice;
        state.refs[c] = refs;
    }
    state.cost = best_cost;
    if best_checkpoint > 0 {
        if let Some(checked) = Active::new(graph, &state.choices) {
            if checked.cost < original_cost {
                *state = checked;
                return true;
            }
        }
        for &(c, choice, refs) in undo[..best_checkpoint].iter().rev() {
            state.choices[c] = choice;
            state.refs[c] = refs;
        }
    }
    state.cost = original_cost;
    false
}

fn repair_exchange(
    graph: &FastGraph<'_>,
    state: &mut Active,
    fallback: &[usize],
    options: &[Vec<usize>],
    scratch: &mut Scratch,
    start: Instant,
    deadline: Duration,
) -> usize {
    let mut commits = 0;
    let mut marks = vec![0usize; options.len()];
    let mut epoch = 0usize;
    let mut undo = Vec::new();
    let mut queue = Vec::new();
    loop {
        let before = commits;
        let mut ranked = Vec::new();
        for n in 0..graph.costs.len() {
            if n & 255 == 0 && start.elapsed() >= deadline {
                break;
            }
            if state.choices[graph.node_class[n]] == NONE {
                continue;
            }
            if let Some(delta) = scratch.price(graph, state, fallback, n, start, deadline) {
                ranked.push((delta, n));
            }
        }
        ranked.sort_by(|a, b| a.0.total_cmp(&b.0));
        for (_, n) in ranked {
            if scratch.budget_expired(start, deadline) {
                break;
            }
            let Some(delta) =
                scratch.evaluate(graph, state, fallback, n, start, deadline, f64::MAX)
            else {
                continue;
            };
            let original_cost = state.cost;
            let pivot = graph.node_class[n];
            undo.clear();
            undo.push((pivot, state.choices[pivot], state.refs[pivot]));
            for &d in &scratch.touched {
                undo.push((d, state.choices[d], state.refs[d]));
            }
            scratch.apply(state, fallback, pivot, n, delta);
            epoch += 1;
            queue.clear();
            for d in scratch
                .touched
                .iter()
                .copied()
                .chain(std::iter::once(pivot))
            {
                if marks[d] != epoch {
                    marks[d] = epoch;
                    queue.push(d);
                }
                for &parent in graph.parents(d) {
                    let c = graph.node_class[parent];
                    if marks[c] != epoch {
                        marks[c] = epoch;
                        queue.push(c);
                    }
                }
            }
            let mut position = 0;
            while position < queue.len() {
                if (position & 63 == 0) && start.elapsed() >= deadline {
                    break;
                }
                let c = queue[position];
                position += 1;
                marks[c] = 0;
                if c == pivot || state.choices[c] == NONE {
                    continue;
                }
                let mut best_replacement = NONE;
                let mut best_delta = 0.0;
                for &replacement in &options[c] {
                    if let Some(delta) = scratch.evaluate(
                        graph,
                        state,
                        fallback,
                        replacement,
                        start,
                        deadline,
                        best_delta,
                    ) {
                        if delta < best_delta {
                            best_delta = delta;
                            best_replacement = replacement;
                        }
                    }
                }
                if best_replacement != NONE {
                    let replacement = best_replacement;
                    let Some(delta) = scratch.evaluate(
                        graph,
                        state,
                        fallback,
                        replacement,
                        start,
                        deadline,
                        -f64::from_bits(1),
                    ) else {
                        continue;
                    };
                    undo.push((c, state.choices[c], state.refs[c]));
                    for &d in &scratch.touched {
                        undo.push((d, state.choices[d], state.refs[d]));
                        if marks[d] != epoch {
                            marks[d] = epoch;
                            queue.push(d);
                        }
                        for &parent in graph.parents(d) {
                            let other = graph.node_class[parent];
                            if marks[other] != epoch {
                                marks[other] = epoch;
                                queue.push(other);
                            }
                        }
                    }
                    scratch.apply(state, fallback, c, replacement, delta);
                }
            }
            if state.cost < original_cost {
                if let Some(checked) = Active::new(graph, &state.choices) {
                    if checked.cost < original_cost {
                        *state = checked;
                        commits += 1;
                        continue;
                    }
                }
            }
            for (c, choice, refs) in undo.drain(..).rev() {
                state.choices[c] = choice;
                state.refs[c] = refs;
            }
            state.cost = original_cost;
        }
        if commits == before || start.elapsed() >= deadline {
            break;
        }
    }
    commits
}

#[derive(Default)]
struct DescentCertificate {
    choices: Vec<usize>,
    fallback: Vec<usize>,
}

impl DescentCertificate {
    fn matches(&self, state: &Active, fallback: &[usize]) -> bool {
        self.choices == state.choices && self.fallback == fallback
    }
    fn record(&mut self, state: &Active, fallback: &[usize]) {
        self.choices.clone_from(&state.choices);
        self.fallback.clear();
        self.fallback.extend_from_slice(fallback);
    }
}

pub(super) fn rooted_choices(
    graph: &FastGraph<'_>,
    choices: &[usize],
) -> Result<Vec<usize>, Error> {
    Active::new(graph, choices)
        .map(|active| active.choices)
        .ok_or_else(|| Error::InvalidSelection("cyclic or incomplete choices".into()))
}

pub(super) fn extract(
    graph: &FastGraph<'_>,
    opts: &Options,
    start: Instant,
) -> Result<Vec<usize>, Error> {
    let seed = graph.seed()?;
    let (mut best, path_admitted) = Active::seed_with_path(graph, &seed)
        .ok_or_else(|| Error::InvalidSelection("productive seed did not cover roots".into()))?;
    let seed_cost = best.cost;
    let deadline = opts.time_limit;
    if best
        .choices
        .iter()
        .enumerate()
        .all(|(c, &n)| n == NONE || graph.class_sizes[c] == 1)
        || best
            .choices
            .iter()
            .all(|&n| n == NONE || graph.costs[n] == 0.0)
        || start.elapsed() >= deadline
    {
        return Ok(best.choices);
    }
    let exact_bound = if path_admitted {
        crate::exact_path::PathBound::new(graph, start, deadline)
    } else {
        None
    };
    if exact_bound
        .as_ref()
        .is_some_and(|bound| bound.matches(&best.choices))
    {
        return Ok(best.choices);
    }
    let classes = graph.class_ids.len();
    let mut options = vec![Vec::new(); classes];
    for (n, &c) in graph.node_class.iter().enumerate() {
        options[c].push(n);
    }
    let mut trajectory = best.choices.clone();
    let mut trajectory_refs = best.refs.clone();
    let mut demand = vec![1.0f64; classes];
    let mut latent = vec![0.0f64; classes];
    let mut seen = vec![NONE; classes];
    let mut scratch = Scratch::new(classes);
    let mut certificate = DescentCertificate::default();
    let mut unchanged = 0;
    let mut stagnant = 0;
    'rounds: for round in 0..32 {
        if start.elapsed() >= deadline {
            break;
        }
        let coefficient = [0.0, 0.5, 1.0, 0.25][round % 4];
        if coefficient > 0.0 {
            latent.fill(0.0);
            seen.fill(NONE);
            let mut work = 0;
            for (c, &chosen) in trajectory.iter().enumerate() {
                if chosen == NONE {
                    continue;
                }
                for &n in &options[c] {
                    for &d in graph.children(n) {
                        work += 1;
                        if work & 1023 == 0 && start.elapsed() >= deadline {
                            break 'rounds;
                        }
                        if d != c && seen[d] != c {
                            seen[d] = c;
                            latent[d] += 1.0;
                        }
                    }
                }
            }
        }
        for c in 0..classes {
            let fresh = (trajectory_refs[c] as f64).max(1.0);
            demand[c] = if round % 4 == 0 {
                fresh
            } else {
                0.35 * demand[c] + 0.65 * fresh
            };
            if coefficient > 0.0 {
                demand[c] += coefficient * latent[c];
            }
        }
        let strength = [0.0, 1.0, 0.5, 1.5, 1.0, 2.0, 0.75, 1.25][round % 8];
        let Some(fallback) = rebuild(graph, &demand, strength, &trajectory, start, deadline) else {
            break;
        };
        let Some(mut candidate) = Active::new(graph, &fallback) else {
            break;
        };
        let certified = certificate.matches(&candidate, &fallback);
        for sweep in 0..if certified { 0 } else { 4 } {
            let before = scratch.moves;
            let offset =
                (opts.seed as usize).wrapping_add(round * 37 + sweep * 17) % classes.max(1);
            let mut completed = true;
            for j in 0..classes {
                if j & 31 == 0 && start.elapsed() >= deadline {
                    completed = false;
                    break;
                }
                let c = (j + offset) % classes;
                if candidate.choices[c] == NONE {
                    continue;
                }
                for &n in &options[c] {
                    scratch.improve(graph, &mut candidate, &fallback, n, start, deadline);
                }
            }
            if completed && before == scratch.moves && start.elapsed() < deadline {
                certificate.record(&candidate, &fallback);
            }
            if before == scratch.moves || start.elapsed() >= deadline {
                break;
            }
        }
        unchanged = if trajectory == candidate.choices {
            unchanged + 1
        } else {
            0
        };
        trajectory.clone_from(&candidate.choices);
        trajectory_refs.clone_from(&candidate.refs);
        if candidate.cost < best.cost {
            best = candidate;
            stagnant = 0;
        } else {
            stagnant += 1;
        }
        if unchanged >= 8 || stagnant >= 8 {
            break;
        }
    }
    let lower_bound = if let Some(bound) = &exact_bound {
        Some(bound.as_float())
    } else if start.elapsed() < deadline {
        path_lower_bound(graph, start, deadline)
    } else {
        None
    };
    if !meets_path_bound(graph, &best, lower_bound) && start.elapsed() < deadline {
        scratch.eligible = Some(prune_equivalent(graph, &mut options, start, deadline));
        scratch.components = dependency_components(graph, &options, start, deadline);
        if let Some(fallback) = rebuild(graph, &demand, 0.0, &best.choices, start, deadline) {
            let mandatory = mandatory_classes(graph, &options, start, deadline);
            let mut potential = vec![0usize; classes];
            let mut counted = vec![NONE; classes];
            for (c, alternatives) in options.iter().enumerate() {
                if best.choices[c] == NONE {
                    continue;
                }
                for &n in alternatives {
                    for &d in graph.children(n) {
                        if counted[d] != c {
                            counted[d] = c;
                            potential[d] += 1;
                        }
                    }
                }
            }
            let mut targets = Vec::new();
            for c in 0..classes {
                if best.choices[c] != NONE && best.refs[c] > 1 && !mandatory[c] {
                    targets.push((c, false, graph.costs[best.choices[c]]));
                } else if best.choices[c] == NONE && potential[c] > 1 && fallback[c] != NONE {
                    targets.push((
                        c,
                        true,
                        (potential[c] - 1) as f64 * graph.costs[fallback[c]],
                    ));
                }
            }
            targets.sort_unstable_by(|a, b| b.2.total_cmp(&a.2).then(a.0.cmp(&b.0)));
            loop {
                let mut improved = false;
                for &(target, opening, _) in &targets {
                    if scratch.budget_expired(start, deadline) {
                        break;
                    }
                    if !opening && (best.choices[target] == NONE || best.refs[target] < 2) {
                        continue;
                    }
                    improved |= if opening {
                        open_support(
                            graph,
                            &mut best,
                            &fallback,
                            &mut scratch,
                            target,
                            start,
                            deadline,
                        )
                    } else {
                        retire_support(
                            graph,
                            &mut best,
                            &fallback,
                            &options,
                            &mut scratch,
                            target,
                            start,
                            deadline,
                        )
                    };
                }
                if !improved || start.elapsed() >= deadline {
                    break;
                }
            }
            if best.cost < seed_cost && start.elapsed() < deadline {
                repair_exchange(
                    graph,
                    &mut best,
                    &fallback,
                    &options,
                    &mut scratch,
                    start,
                    deadline,
                );
            }
        }
    }
    Ok(best.choices)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ClassId, EGraph, NodeId};
    use egraph_serialize::Node;
    fn class_id(s: &str) -> ClassId {
        s.into()
    }
    fn node_id(s: &str) -> NodeId {
        s.into()
    }
    fn options() -> Options {
        Options {
            time_limit: Duration::from_secs(60),
            seed: 0,
        }
    }
    fn fixture(nodes: &[(&str, &str, f64, &[&str])], roots: &[&str]) -> EGraph {
        let mut graph = EGraph::default();
        for &(id, class, cost, children) in nodes {
            graph.add_node(
                id,
                Node {
                    op: id.into(),
                    eclass: class_id(class),
                    cost: Cost::new(cost).unwrap(),
                    children: children.iter().map(|c| node_id(c)).collect(),
                    subsumed: false,
                },
            );
        }
        graph.root_eclasses = roots.iter().map(|r| class_id(r)).collect();
        graph
    }

    #[test]
    fn certificate_invalidates_on_inactive_fallback_change() {
        let state = Active {
            choices: vec![0, NONE],
            refs: vec![1, 0],
            cost: 1.0,
        };
        let mut certificate = DescentCertificate::default();
        certificate.record(&state, &[0, 2]);
        assert!(certificate.matches(&state, &[0, 2]));
        assert!(!certificate.matches(&state, &[0, 3]));
        let changed = Active {
            choices: vec![1, NONE],
            ..state.clone()
        };
        assert!(!certificate.matches(&changed, &[0, 2]));
    }

    #[test]
    fn reconstruction_keeps_best_ready_alternative_and_preferred_ties() {
        let input = fixture(
            &[
                ("r", "r", 0.0, &["a", "a"]),
                ("a", "a", 9.0, &[]),
                ("cheap", "a", 2.0, &[]),
                ("preferred", "a", 2.0, &[]),
                ("cycle", "a", 0.0, &["r"]),
                ("later", "a", 0.0, &["b"]),
                ("b", "b", 1.0, &[]),
            ],
            &["r"],
        );
        let opts = options();
        let start = Instant::now();
        let graph = FastGraph::new(&input, &input.root_eclasses).unwrap_or_else(|_| panic!());
        let node = |id: &str| input.nodes.get_index_of(&node_id(id)).unwrap();
        let mut preferred = vec![NONE; graph.class_ids.len()];
        preferred[graph.node_class[node("a")]] = node("preferred");
        let mut demand = vec![1.0; preferred.len()];
        let choices = rebuild(&graph, &demand, 0.0, &preferred, start, opts.time_limit).unwrap();
        assert_eq!(choices[graph.node_class[node("a")]], node("later"));
        assert_eq!(Active::new(&graph, &choices).unwrap().cost, 1.0);
        demand[graph.node_class[node("a")]] = 2.0;
        let choices = rebuild(&graph, &demand, 1.0, &preferred, start, opts.time_limit).unwrap();
        assert_eq!(choices[graph.node_class[node("a")]], node("preferred"));
        assert!(Active::new(&graph, &choices).is_some());
    }

    #[test]
    fn forced_class_pruning_uses_all_alternatives_not_only_the_incumbent() {
        let input = fixture(
            &[
                ("r", "r", 0.0, &["a", "s"]),
                ("rx", "r", 0.0, &["a"]),
                ("a", "a", 0.0, &["t"]),
                ("ax", "a", 0.0, &["t", "s"]),
                ("t", "t", 1.0, &[]),
                ("s", "s", 1.0, &[]),
            ],
            &["r"],
        );
        let opts = options();
        let start = Instant::now();
        let graph = FastGraph::new(&input, &input.root_eclasses).unwrap_or_else(|_| panic!());
        let mut choices = vec![Vec::new(); graph.class_ids.len()];
        for (n, &c) in graph.node_class.iter().enumerate() {
            choices[c].push(n);
        }
        let required = mandatory_classes(&graph, &choices, start, opts.time_limit);
        for id in ["r", "a", "t", "s"] {
            let n = input.nodes.get_index_of(&node_id(id)).unwrap();
            assert_eq!(required[graph.node_class[n]], id != "s");
        }
    }

    #[test]
    fn atomic_opening_retains_profitable_prefix_and_rolls_back_failures() {
        for support_cost in [5.0, 20.0] {
            let input = fixture(
                &[
                    ("r", "r", 0.0, &["a", "b", "c"]),
                    ("a", "a", 4.0, &[]),
                    ("ax", "a", 0.0, &["s", "s"]),
                    ("b", "b", 4.0, &[]),
                    ("bx", "b", 0.0, &["s"]),
                    ("c", "c", 1.0, &[]),
                    ("cx", "c", 3.0, &["s"]),
                    ("s", "s", support_cost, &[]),
                    ("sc", "s", 0.0, &["r"]),
                ],
                &["r"],
            );
            let opts = options();
            let start = Instant::now();
            let graph = FastGraph::new(&input, &input.root_eclasses).unwrap_or_else(|_| panic!());
            let mut choices = vec![NONE; graph.class_ids.len()];
            for id in ["r", "a", "b", "c"] {
                let n = input.nodes.get_index_of(&node_id(id)).unwrap();
                choices[graph.node_class[n]] = n;
            }
            let mut fallback = choices.clone();
            let n = input.nodes.get_index_of(&node_id("s")).unwrap();
            let target = graph.node_class[n];
            fallback[target] = n;
            let mut state = Active::new(&graph, &choices).unwrap();
            let original = state.clone();
            let mut scratch = Scratch::new(choices.len());
            assert_eq!(
                open_support(
                    &graph,
                    &mut state,
                    &fallback,
                    &mut scratch,
                    target,
                    start,
                    opts.time_limit
                ),
                support_cost == 5.0
            );
            let checked = Active::new(&graph, &state.choices).unwrap();
            assert_eq!(state.refs, checked.refs);
            assert_eq!(state.cost, checked.cost);
            if support_cost == 5.0 {
                assert_eq!(state.cost, 6.0);
                assert_eq!(
                    state.choices
                        [graph.node_class[input.nodes.get_index_of(&node_id("c")).unwrap()]],
                    input.nodes.get_index_of(&node_id("c")).unwrap()
                );
            } else {
                assert_eq!(state.choices, original.choices);
                assert_eq!(state.refs, original.refs);
                assert_eq!(state.cost, original.cost);
            }
        }
    }

    #[test]
    fn atomic_opening_skips_cheap_cyclic_proposal() {
        let input = fixture(
            &[
                ("r", "r", 0.0, &["a", "b"]),
                ("a", "a", 4.0, &[]),
                ("cycle", "a", 0.0, &["s", "r"]),
                ("ax", "a", 1.0, &["s"]),
                ("b", "b", 4.0, &[]),
                ("bx", "b", 0.0, &["s"]),
                ("s", "s", 5.0, &[]),
            ],
            &["r"],
        );
        let opts = options();
        let start = Instant::now();
        let graph = FastGraph::new(&input, &input.root_eclasses).unwrap_or_else(|_| panic!());
        let mut choices = vec![NONE; graph.class_ids.len()];
        for id in ["r", "a", "b"] {
            let n = input.nodes.get_index_of(&node_id(id)).unwrap();
            choices[graph.node_class[n]] = n;
        }
        let mut fallback = choices.clone();
        let support = input.nodes.get_index_of(&node_id("s")).unwrap();
        let target = graph.node_class[support];
        fallback[target] = support;
        let mut state = Active::new(&graph, &choices).unwrap();
        let mut scratch = Scratch::new(choices.len());
        assert!(open_support(
            &graph,
            &mut state,
            &fallback,
            &mut scratch,
            target,
            start,
            opts.time_limit
        ));
        let checked = Active::new(&graph, &state.choices).unwrap();
        assert_eq!(state.cost, 6.0);
        assert_eq!(state.cost, checked.cost);
        assert_eq!(state.refs, checked.refs);
    }

    #[test]
    fn equivalent_support_pruning_keeps_cheapest_per_class_and_child_set() {
        let input = fixture(
            &[
                ("r", "r", 3.0, &["a", "b"]),
                ("rx", "r", 1.0, &["b", "a", "a"]),
                ("ry", "r", 2.0, &["a"]),
                ("a", "a", 4.0, &[]),
                ("ax", "a", 2.0, &[]),
                ("b", "b", 1.0, &[]),
            ],
            &["r"],
        );
        let opts = options();
        let start = Instant::now();
        let graph = FastGraph::new(&input, &input.root_eclasses).unwrap_or_else(|_| panic!());
        let mut alternatives = vec![Vec::new(); graph.class_ids.len()];
        for (n, &c) in graph.node_class.iter().enumerate() {
            alternatives[c].push(n);
        }
        let eligible = prune_equivalent(&graph, &mut alternatives, start, opts.time_limit);
        for id in ["r", "rx", "ry", "a", "ax", "b"] {
            let n = input.nodes.get_index_of(&node_id(id)).unwrap();
            assert_eq!(eligible[n], !matches!(id, "r" | "a"));
        }
    }

    #[test]
    fn path_bound_handles_sharing_fractional_costs_and_cycles() {
        for cost in [5.0, 0.1] {
            let input = fixture(
                &[
                    ("r", "r", 0.0, &["a", "b"]),
                    ("a", "a", 0.0, &["s"]),
                    ("b", "b", 0.0, &["s"]),
                    ("s", "s", cost, &[]),
                    ("cycle", "s", 0.0, &["r"]),
                ],
                &["r"],
            );
            let opts = options();
            let start = Instant::now();
            let graph = FastGraph::new(&input, &input.root_eclasses).unwrap_or_else(|_| panic!());
            let lower = path_lower_bound(&graph, start, opts.time_limit).unwrap();
            assert_eq!(lower, cost);
            let seed = graph.seed().unwrap();
            let state = Active::new(&graph, &seed).unwrap();
            assert!(meets_path_bound(&graph, &state, Some(lower)));
            assert!(!meets_path_bound(&graph, &state, Some(lower / 2.0)));
        }
    }

    #[test]
    fn component_pruning_preserves_forbidden_support_checks() {
        let input = fixture(
            &[
                ("r", "r", 5.0, &[]),
                ("rx", "r", 0.0, &["a"]),
                ("a", "a", 1.0, &["b"]),
                ("ax", "a", 1.0, &[]),
                ("b", "b", 1.0, &["a"]),
                ("bx", "b", 1.0, &[]),
            ],
            &["r"],
        );
        let opts = options();
        let start = Instant::now();
        let graph = FastGraph::new(&input, &input.root_eclasses).unwrap_or_else(|_| panic!());
        let mut alternatives = vec![Vec::new(); graph.class_ids.len()];
        for (n, &c) in graph.node_class.iter().enumerate() {
            alternatives[c].push(n);
        }
        let ids = dependency_components(&graph, &alternatives, start, opts.time_limit).unwrap();
        let node = |id: &str| input.nodes.get_index_of(&node_id(id)).unwrap();
        let class = |id: &str| graph.node_class[node(id)];
        assert_eq!(ids[class("a")], ids[class("b")]);
        assert_ne!(ids[class("r")], ids[class("a")]);
        let mut choices = vec![NONE; alternatives.len()];
        for id in ["r", "a", "bx"] {
            choices[class(id)] = node(id);
        }
        let state = Active::new(&graph, &choices).unwrap();
        let mut scratch = Scratch::new(choices.len());
        scratch.components = Some(ids);
        assert!(scratch
            .evaluate(
                &graph,
                &state,
                &choices,
                node("rx"),
                start,
                opts.time_limit,
                0.0
            )
            .is_some());
        assert!(scratch
            .evaluate_avoiding(
                &graph,
                &state,
                &choices,
                node("rx"),
                start,
                opts.time_limit,
                0.0,
                Some(class("b"))
            )
            .is_none());
    }

    #[test]
    fn dependency_repair_commits_joint_saving_or_restores_state() {
        for alternative_cost in [3.0, 6.0] {
            let input = fixture(
                &[
                    ("r", "r", 0.0, &["a", "b"]),
                    ("a", "a", 0.0, &["s"]),
                    ("ax", "a", alternative_cost, &[]),
                    ("b", "b", 0.0, &["s"]),
                    ("bx", "b", alternative_cost, &[]),
                    ("c", "c", 0.0, &["s"]),
                    ("cx", "c", alternative_cost, &[]),
                    ("s", "s", 10.0, &[]),
                    ("cycle", "s", 0.0, &["r"]),
                ],
                &["r"],
            );
            let opts = options();
            let start = Instant::now();
            let graph = FastGraph::new(&input, &input.root_eclasses).unwrap_or_else(|_| panic!());
            let mut choices = vec![NONE; graph.class_ids.len()];
            let mut alternatives = vec![Vec::new(); choices.len()];
            for (n, &c) in graph.node_class.iter().enumerate() {
                alternatives[c].push(n);
            }
            for id in ["r", "a", "b", "c", "s"] {
                let n = input.nodes.get_index_of(&node_id(id)).unwrap();
                choices[graph.node_class[n]] = n;
            }
            let fallback = choices.clone();
            let mut state = Active::new(&graph, &choices).unwrap();
            let original = state.clone();
            let mut scratch = Scratch::new(choices.len());
            repair_exchange(
                &graph,
                &mut state,
                &fallback,
                &alternatives,
                &mut scratch,
                start,
                opts.time_limit,
            );
            let checked = Active::new(&graph, &state.choices).unwrap();
            assert_eq!(state.cost, checked.cost);
            assert_eq!(state.refs, checked.refs);
            assert_eq!(state.cost, if alternative_cost == 3.0 { 6.0 } else { 10.0 });
            if alternative_cost == 6.0 {
                assert_eq!(state.choices, original.choices);
                assert_eq!(state.refs, original.refs);
            }
            let before = state.clone();
            assert_eq!(
                repair_exchange(
                    &graph,
                    &mut state,
                    &fallback,
                    &alternatives,
                    &mut scratch,
                    start,
                    Duration::ZERO
                ),
                0
            );
            assert_eq!(state.choices, before.choices);
            assert_eq!(state.refs, before.refs);
            assert_eq!(state.cost, before.cost);
        }
    }

    #[test]
    fn atomic_retirement_crosses_barrier_or_restores_exact_state() {
        for cost in [3.0, 4.0] {
            let input = fixture(
                &[
                    ("r", "r", 0.0, &["a", "b", "c"]),
                    ("a", "a", 0.0, &["s", "s"]),
                    ("ax", "a", cost, &[]),
                    ("b", "b", 0.0, &["s"]),
                    ("bx", "b", cost, &[]),
                    ("c", "c", 0.0, &["s"]),
                    ("cx", "c", cost, &[]),
                    ("s", "s", 10.0, &[]),
                    ("sc", "s", 0.0, &["r"]),
                ],
                &["r"],
            );
            let opts = options();
            let start = Instant::now();
            let graph = FastGraph::new(&input, &input.root_eclasses).unwrap_or_else(|_| panic!());
            let mut choices = vec![NONE; graph.class_ids.len()];
            let mut options = vec![Vec::new(); choices.len()];
            for (n, &c) in graph.node_class.iter().enumerate() {
                options[c].push(n);
            }
            for id in ["r", "a", "b", "c", "s"] {
                let n = input.nodes.get_index_of(&node_id(id)).unwrap();
                choices[graph.node_class[n]] = n;
            }
            let fallback = rebuild(
                &graph,
                &vec![1.0; choices.len()],
                0.0,
                &choices,
                start,
                opts.time_limit,
            )
            .unwrap();
            let mut state = Active::new(&graph, &choices).unwrap();
            let original = state.clone();
            let target = graph.node_class[input.nodes.get_index_of(&node_id("s")).unwrap()];
            let mut scratch = Scratch::new(choices.len());
            assert_eq!(
                retire_support(
                    &graph,
                    &mut state,
                    &fallback,
                    &options,
                    &mut scratch,
                    target,
                    start,
                    opts.time_limit
                ),
                cost == 3.0
            );
            let checked = Active::new(&graph, &state.choices).unwrap();
            assert_eq!(state.refs, checked.refs);
            assert_eq!(state.cost, checked.cost);
            if cost == 3.0 {
                assert_eq!(state.cost, 9.0);
                assert_eq!(state.refs[target], 0);
            } else {
                assert_eq!(state.choices, original.choices);
                assert_eq!(state.refs, original.refs);
                assert_eq!(state.cost, original.cost);
            }
            let before = state.clone();
            assert!(!retire_support(
                &graph,
                &mut state,
                &fallback,
                &options,
                &mut scratch,
                target,
                start,
                Duration::ZERO
            ));
            assert_eq!(state.choices, before.choices);
            assert_eq!(state.refs, before.refs);
            assert_eq!(state.cost, before.cost);
        }
    }

    #[test]
    fn sparse_trial_retains_single_integer_saving_at_large_total() {
        let input = fixture(
            &[
                ("a", "a", 1_000_000_000_001.0, &[]),
                ("b", "a", 1_000_000_000_000.0, &[]),
            ],
            &["a"],
        );
        let opts = options();
        let start = Instant::now();
        let graph = FastGraph::new(&input, &input.root_eclasses)
            .unwrap_or_else(|_| panic!("invalid fixture"));
        let a = input.nodes.get_index_of(&node_id("a")).unwrap();
        let b = input.nodes.get_index_of(&node_id("b")).unwrap();
        let mut state = Active::new(&graph, &[a]).unwrap();
        let mut scratch = Scratch::new(1);
        assert!(scratch.improve(&graph, &mut state, &[b], b, start, opts.time_limit));
        assert_eq!(state.cost, 1_000_000_000_000.0);
    }
}
