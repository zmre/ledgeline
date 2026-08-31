//! Market-price database + valuation — port of
//! `web/src/lib/reports/prices.ts`, extended to hledger's transitive price
//! graph.
//!
//! Valuation walks the market-price GRAPH in effect at `as_of` rather than
//! looking for a single direct price, mirroring
//! `Hledger.Data.Valuation.priceLookup`: the shortest chain of *forward* prices
//! (`P`-declared or cost-inferred) from the commodity to the target, and only if
//! there is none, the shortest chain that may additionally traverse *reversed*
//! edges. Commodities from which the target is unreachable are SKIPPED (never
//! guessed) and reported via the optional [`ValuationMeta`] out-param.
//!
//! See [`PriceGraph`] for the edge set, edge ordering and tie-breaking rules,
//! all of which are held to hledger's behaviour.

use super::mixed_amount::MixedAmount;
use crate::decimal::{Dec, DecError, MAX_PARSE_PLACES};
use crate::model::{Amount, Commodity, CostKind, PriceDirective, Transaction};
use std::collections::{BTreeMap, BTreeSet};

/// A market-price lookup table built from `P` directives.
#[derive(Debug, Clone)]
pub struct PriceDb {
    /// Directives per priced commodity, stable-sorted ascending by date (so the
    /// last-declared wins for equal dates on the reverse scan).
    by_commodity: BTreeMap<Commodity, Vec<PriceDirective>>,
    /// Every commodity something is priced IN, ranked most-frequent first with
    /// lexical ties — the candidate valuation targets. Empty when there are no
    /// directives.
    targets: Vec<Commodity>,
}

impl PriceDb {
    /// Build a [`PriceDb`] from directives (in journal/declaration order).
    #[must_use]
    pub fn build(directives: &[PriceDirective]) -> PriceDb {
        let mut by_commodity: BTreeMap<Commodity, Vec<PriceDirective>> = BTreeMap::new();
        for directive in directives {
            by_commodity
                .entry(directive.commodity.clone())
                .or_default()
                .push(directive.clone());
        }
        // Stable sort: same-date directives keep journal order.
        for list in by_commodity.values_mut() {
            list.sort_by(|a, b| a.date.cmp(&b.date));
        }

        // Candidate valuation targets, ranked by how often something is priced in
        // them. A `BTreeMap`'s keys are already lexical, so a STABLE sort by
        // descending count leaves ties in lexical order — the same
        // most-frequent-then-lexical winner the old running-argmax loop picked,
        // now with the runners-up kept (see [`PriceDb::base_candidates`]).
        let mut counts: BTreeMap<&Commodity, usize> = BTreeMap::new();
        for directive in directives {
            *counts.entry(&directive.price.commodity).or_insert(0) += 1;
        }
        let mut targets: Vec<Commodity> = counts.keys().map(|target| (*target).clone()).collect();
        targets.sort_by_key(|target| std::cmp::Reverse(counts[target]));

        PriceDb {
            by_commodity,
            targets,
        }
    }

    /// The latest directive for `commodity` dated ≤ `as_of` that also satisfies
    /// `matches`, scanning newest-first (last-declared wins on ties).
    fn latest(
        &self,
        commodity: &Commodity,
        as_of: &str,
        matches: impl Fn(&PriceDirective) -> bool,
    ) -> Option<&Amount> {
        in_effect(self.by_commodity.get(commodity)?, as_of)
            .iter()
            .rev()
            .find(|directive| matches(directive))
            .map(|directive| &directive.price)
    }

    /// Latest `P` directive for `commodity` dated ≤ `as_of`, regardless of the
    /// target it is priced in.
    #[must_use]
    pub fn lookup(&self, commodity: &Commodity, as_of: &str) -> Option<&Amount> {
        self.latest(commodity, as_of, |_| true)
    }

    /// Latest `P` directive dated ≤ `as_of` pricing `commodity` directly in
    /// `target`.
    #[must_use]
    pub fn lookup_in(
        &self,
        commodity: &Commodity,
        target: &Commodity,
        as_of: &str,
    ) -> Option<&Amount> {
        self.latest(commodity, as_of, |directive| {
            &directive.price.commodity == target
        })
    }

    /// Default valuation target (most frequent price commodity; `None` when
    /// there are no directives).
    #[must_use]
    pub fn base_commodity(&self) -> Option<&Commodity> {
        self.targets.first()
    }

    /// Every commodity something is priced IN, most frequent first (lexical
    /// ties) — [`PriceDb::base_commodity`] is just the head of this list.
    ///
    /// Frequency alone is a poor way to pick ONE valuation commodity, because it
    /// has no idea what is being valued: three jotted-down `P … EUR` travel
    /// cross-rates outvote the single `P VTI $120.00` that prices an entire
    /// portfolio, and the portfolio then reads as zero because nothing connects
    /// VTI to EUR (HOLD-3). Callers that know what they need to value should walk
    /// these candidates and take the first one that actually prices it — see
    /// `holdings::engine::choose_base`. hledger sidesteps the question by valuing
    /// each commodity in ITS OWN latest price target rather than choosing a
    /// single base at all.
    #[must_use]
    pub fn base_candidates(&self) -> &[Commodity] {
        &self.targets
    }

    /// The market-price graph in effect at `as_of` — hledger's
    /// `makePriceGraph`. Build it once and reuse it across commodities.
    #[must_use]
    pub fn graph_at(&self, as_of: &str) -> PriceGraph<'_> {
        // hledger indexes prices by (from, to) PAIR and takes each pair's
        // latest entry ≤ the date, so at most one edge exists per directed pair.
        // Iterating the date-sorted list and letting later entries overwrite
        // reproduces that, including "explicit wins a same-date tie" (callers
        // append the explicit directives after the inferred ones).
        //
        // The edge ORDER is load-bearing for tie-breaking (see [`PriceGraph`]):
        // hledger walks `M.elems` of that pair-keyed map, i.e. ascending by
        // `(from, to)`. A `BTreeMap` of commodities keyed by `from`, each
        // holding a `BTreeMap` keyed by `to`, yields exactly that order.
        let forward: Vec<Edge<'_>> = self
            .by_commodity
            .iter()
            .flat_map(|(from, directives)| {
                let latest: BTreeMap<&Commodity, Dec> = in_effect(directives, as_of)
                    .iter()
                    .map(|directive| (&directive.price.commodity, directive.price.quantity))
                    .collect();
                latest
                    .into_iter()
                    .map(move |(to, rate)| Edge { from, to, rate })
            })
            .collect();

        // Reverse edges for pairs with no forward edge of their own, appended in
        // forward order (hledger: `forwardprices ++ reverseprices`).
        let forward_pairs: BTreeSet<(&Commodity, &Commodity)> =
            forward.iter().map(|edge| (edge.from, edge.to)).collect();
        let all: Vec<Edge<'_>> = forward
            .iter()
            .copied()
            .chain(
                forward
                    .iter()
                    .filter(|edge| !forward_pairs.contains(&(edge.to, edge.from)))
                    .filter_map(|edge| {
                        reverse_rate(edge.rate).map(|rate| Edge {
                            from: edge.to,
                            to: edge.from,
                            rate,
                        })
                    }),
            )
            .collect();

        PriceGraph { forward, all }
    }
}

/// The prefix of one commodity's directives that is IN EFFECT at `as_of` — every
/// entry dated ≤ it.
///
/// The list is stable-sorted ascending by date ([`PriceDb::build`]), so the
/// in-effect entries are exactly a prefix and its length is a binary search
/// rather than a scan. That is the whole of PERF-5d: [`PriceDb::latest`] used to
/// filter on the date while walking backwards from the newest directive, which
/// costs `O(1)` for a "value it as of today" query and `O(N)` for a historical
/// one — and a net-worth or holdings series values a decades-long journal at
/// every bucket, so the historical case is the *common* one. Same element
/// chosen, same tie-breaking: the caller still scans this prefix newest-first,
/// so a later same-date directive still wins.
///
/// The newest-directive test in front is not an optimization of the search but a
/// guard against making the OTHER query worse: "value this as of today" needs no
/// search at all, since every directive is already in effect, and going straight
/// to `partition_point` would charge it log₂(N) date compares for an answer that
/// one compare settles. With the guard, both ends of the range are cheap.
fn in_effect<'a>(directives: &'a [PriceDirective], as_of: &str) -> &'a [PriceDirective] {
    if directives
        .last()
        .is_some_and(|newest| newest.date.as_str() <= as_of)
    {
        return directives;
    }
    &directives[..directives.partition_point(|directive| directive.date.as_str() <= as_of)]
}

/// One directed edge of the price graph: one unit of `from` is worth `rate` of
/// `to`.
#[derive(Debug, Clone, Copy)]
struct Edge<'a> {
    from: &'a Commodity,
    to: &'a Commodity,
    rate: Dec,
}

/// The market-price graph in effect on a single date — hledger's `PriceGraph`,
/// built by [`PriceDb::graph_at`].
///
/// hledger's preference order, which [`PriceGraph::rate`] reproduces:
///
/// 1. the shortest chain of *forward* edges (a declared or inferred `P`), then
/// 2. the shortest chain that may also use *reverse* edges (`1/rate`, added only
///    for pairs that have no forward edge of their own).
///
/// A one-hop chain is just a direct price, so this subsumes the old direct-only
/// lookup; and because a forward chain of ANY length is preferred over a reverse
/// edge, `A→B→C` beats `1/(C→A)`.
///
/// Ties between equal-length chains resolve exactly as hledger's
/// `pricesShortestPath` does: edges are ordered by `(from, to)` ascending with
/// every reverse edge after every forward one (each in its originating forward
/// edge's position), the breadth-first frontier is extended in order, and the
/// left-most complete path at the first complete length wins. Verified against
/// hledger 1.52 — see `tests/reports_prices.rs`.
#[derive(Debug, Clone)]
pub struct PriceGraph<'a> {
    /// Declared/inferred edges, at most one per directed pair, ordered by
    /// `(from, to)`.
    forward: Vec<Edge<'a>>,
    /// `forward`, then the reversed edges of pairs that have no forward edge.
    all: Vec<Edge<'a>>,
}

impl PriceGraph<'_> {
    /// What one unit of `from` is worth in `to`, or `None` when no chain of
    /// prices connects them (or when they are the same commodity, which hledger
    /// also reports as "no price" — the caller treats it as identity).
    ///
    /// # Errors
    /// Returns [`DecError`] on decimal overflow while combining a chain.
    pub fn rate(&self, from: &Commodity, to: &Commodity) -> Result<Option<Dec>, DecError> {
        if from == to {
            return Ok(None);
        }
        let Some(chain) =
            shortest_path(from, to, &self.forward).or_else(|| shortest_path(from, to, &self.all))
        else {
            return Ok(None);
        };
        chain_rate(&chain).map(Some)
    }
}

/// The commodities reachable from `start` over `edges` (including `start`).
///
/// A cheap O(V+E) guard in front of the exhaustive search below. hledger's
/// breadth-first search enumerates every simple path out of `start` before
/// concluding there is none, which is the *common* case here — a genuinely
/// unpriced commodity, hit once per account per period. Skipping it when the
/// target is unreachable is semantics-preserving (an unreachable target has no
/// path by definition) and keeps a densely connected FX graph from turning one
/// unpriced commodity into a combinatorial blow-up.
fn reachable<'a>(start: &'a Commodity, edges: &[Edge<'a>]) -> BTreeSet<&'a Commodity> {
    let mut seen: BTreeSet<&Commodity> = BTreeSet::from([start]);
    let mut pending: Vec<&Commodity> = vec![start];
    while let Some(node) = pending.pop() {
        for edge in edges.iter().filter(|edge| edge.from == node) {
            if seen.insert(edge.to) {
                pending.push(edge.to);
            }
        }
    }
    seen
}

/// A partially explored chain: indices into the edge slice, plus the edges still
/// usable on this branch (in slice order).
struct Partial {
    path: Vec<usize>,
    unused: Vec<usize>,
}

/// The rates along the shortest chain of `edges` from `start` to `end`, or
/// `None` when there is none — a port of hledger's `pricesShortestPath`.
///
/// Breadth-first, extending the whole frontier one hop at a time and stopping at
/// the left-most complete path of the first complete length. A branch never
/// revisits a commodity (hledger drops every edge pointing back into the path's
/// nodes), so a cycle in the price graph cannot loop forever: chains are simple,
/// hence at most one hop shorter than the commodity count.
fn shortest_path(start: &Commodity, end: &Commodity, edges: &[Edge<'_>]) -> Option<Vec<Dec>> {
    if !reachable(start, edges).contains(end) {
        return None;
    }
    let mut frontier = vec![Partial {
        path: Vec::new(),
        unused: (0..edges.len()).collect(),
    }];
    loop {
        let mut extended: Vec<Partial> = Vec::new();
        for partial in &frontier {
            let path_end = partial.path.last().map_or(start, |&i| edges[i].to);
            let (out, rest): (Vec<usize>, Vec<usize>) = partial
                .unused
                .iter()
                .copied()
                .partition(|&i| edges[i].from == path_end);
            for step in out {
                let path: Vec<usize> = partial.path.iter().copied().chain([step]).collect();
                let visited: Vec<&Commodity> = std::iter::once(start)
                    .chain(path.iter().map(|&i| edges[i].to))
                    .collect();
                let unused = rest
                    .iter()
                    .copied()
                    .filter(|&i| !visited.contains(&edges[i].to))
                    .collect();
                extended.push(Partial { path, unused });
            }
        }
        if extended.is_empty() {
            return None;
        }
        if let Some(complete) = extended
            .iter()
            .find(|partial| partial.path.last().is_some_and(|&i| edges[i].to == end))
        {
            return Some(complete.path.iter().map(|&i| edges[i].rate).collect());
        }
        frontier = extended;
    }
}

/// Maximum fractional places kept for a rate we DERIVE (a reversed edge or a
/// multi-hop chain).
///
/// hledger's rates are `Data.Decimal`s, so its `1/rate` saturates at that type's
/// 255-place ceiling — effectively exact — and is merely *displayed* at
/// `defaultMaxPrecision` (8). `Dec` is `i128`-backed, so neither an unbounded
/// reciprocal nor an unbounded chain product is representable. We keep
/// [`MAX_PARSE_PLACES`] places, the most a price can carry once parsed, which is
/// finer than anything hledger prints, and round half-even beyond that.
const MAX_RATE_PLACES: u32 = MAX_PARSE_PLACES;

/// Combine a chain's edge rates into one rate, as hledger does: the exact
/// product (its `Decimal` multiply normalizes, stripping trailing zeros) padded
/// back out to the widest scale seen among the edges (`setMinDecimalPlaces`).
///
/// ROUNDING HAPPENS HERE, once per hop, and only above
/// `max(MAX_RATE_PLACES, widest edge scale)`. A single-edge chain therefore
/// returns that edge's rate bit-for-bit (normalizing then padding back to its own
/// scale is the identity), so direct valuation is unchanged; only chains, whose
/// scales would otherwise add up and overflow `i128`, are capped.
fn chain_rate(rates: &[Dec]) -> Result<Dec, DecError> {
    let min_places = rates.iter().map(|rate| rate.places).max().unwrap_or(0);
    let max_places = min_places.max(MAX_RATE_PLACES);
    let product = rates.iter().try_fold(Dec::new(1, 0), |acc, rate| {
        capped(acc.mul(*rate)?, max_places)
    })?;
    padded(product, min_places)
}

/// Round `value` half-even down to at most `max_places` fractional places.
fn capped(value: Dec, max_places: u32) -> Result<Dec, DecError> {
    if value.places <= max_places {
        return Ok(value);
    }
    let divisor = pow10(value.places - max_places)?;
    Ok(Dec::new(
        div_round_half_even(value.mantissa, divisor)?,
        max_places,
    ))
}

/// Pad `value` with trailing zeros up to `min_places` fractional places.
fn padded(value: Dec, min_places: u32) -> Result<Dec, DecError> {
    if value.places >= min_places {
        return Ok(value);
    }
    let factor = pow10(min_places - value.places)?;
    Ok(Dec::new(
        value
            .mantissa
            .checked_mul(factor)
            .ok_or(DecError::Overflow)?,
        min_places,
    ))
}

/// The rate of a reversed edge: `1/unit`, exact when that terminates within
/// [`MAX_RATE_PLACES`] places and rounded half-even to that many otherwise.
///
/// This is what makes a commodity priced only as a cost/price DENOMINATOR
/// valuable at all. [`exact_reciprocal`] alone is not enough: `1/220` has a
/// prime factor of 11 and never terminates, so `10 AAPL @ $220.00` used to
/// leave the `-$2,200.00` cash leg unvalued. hledger reverses a zero rate to
/// zero rather than dropping the edge; we match that.
fn reverse_rate(unit: Dec) -> Option<Dec> {
    if unit.mantissa == 0 {
        return Some(Dec::zero());
    }
    if let Some(exact) = exact_reciprocal(unit)
        && exact.places <= MAX_RATE_PLACES
    {
        return Some(exact);
    }
    // 1/unit = 10^unit.places / |unit.mantissa|, scaled by 10^MAX_RATE_PLACES so
    // the single half-even division lands on that many fractional places.
    let sign = unit.mantissa.signum();
    let magnitude = unit.mantissa.checked_abs()?;
    let numerator = pow10(unit.places.checked_add(MAX_RATE_PLACES)?).ok()?;
    let quotient = div_round_half_even(numerator, magnitude).ok()?;
    Some(Dec::new(sign.checked_mul(quotient)?, MAX_RATE_PLACES))
}

/// Out-param for [`value_at`]: commodities that had to be skipped (deduped, in
/// encounter order — which, over a `BTreeMap`, is lexical).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ValuationMeta {
    /// Commodities from which no chain of prices reaches the target at `as_of`.
    pub unpriced: Vec<Commodity>,
}

/// Non-normalizing exact multiply, mirroring `money.ts`'s `mul` (`m·m`, `p+p`,
/// NO trailing-zero stripping). The engine's canonical `Dec::mul` normalizes to
/// match hledger's parser; valuation must keep the TS representation so ported
/// expectations line up bit-for-bit.
///
/// `pub(crate)` so the holdings engine (`crate::holdings`) reuses the exact same
/// non-normalizing multiply for its basis/market-value math instead of
/// duplicating it.
pub(crate) fn mul_raw(a: Dec, b: Dec) -> Result<Dec, DecError> {
    let mantissa = a
        .mantissa
        .checked_mul(b.mantissa)
        .ok_or(DecError::Overflow)?;
    let places = a.places.checked_add(b.places).ok_or(DecError::Overflow)?;
    Ok(Dec::new(mantissa, places))
}

/// `10^exp` as an `i128`, checked for overflow (mirrors `decimal::pow10`, which
/// is private to that module).
pub(crate) fn pow10(exp: u32) -> Result<i128, DecError> {
    10i128.checked_pow(exp).ok_or(DecError::Overflow)
}

/// Rounded division, half-even (banker's rounding) — port of the TS
/// `divRoundHalfEven`. `domain/money` has no `Dec` division on purpose; this is
/// the one place price/holdings math needs it.
///
/// The denominator is always positive at every call site (a share count or a
/// `|qty|`); a zero denominator is unreachable and is surfaced as the same
/// never-unwrapped overflow arm rather than panicking.
pub(crate) fn div_round_half_even(numerator: i128, denominator: i128) -> Result<i128, DecError> {
    if denominator == 0 {
        return Err(DecError::Overflow);
    }
    let negative = (numerator < 0) != (denominator < 0);
    let n = numerator.checked_abs().ok_or(DecError::Overflow)?;
    let d = denominator.checked_abs().ok_or(DecError::Overflow)?;
    let mut q = n / d;
    let r = n % d;
    let twice = r.checked_mul(2).ok_or(DecError::Overflow)?;
    if twice > d || (twice == d && q % 2 == 1) {
        q = q.checked_add(1).ok_or(DecError::Overflow)?;
    }
    Ok(if negative { -q } else { q })
}

/// Per-unit price from a `@@` total: `total / |qty|`, rounded half-even to
/// `total.p + qty.p` decimal places (port of the TS `perUnitFromTotal`). Shared
/// by the holdings engine and net-worth cost inference.
pub(crate) fn per_unit_from_total(total: Dec, qty: Dec) -> Result<Dec, DecError> {
    let places = total
        .places
        .checked_add(qty.places)
        .ok_or(DecError::Overflow)?;
    let factor = pow10(qty.places.checked_mul(2).ok_or(DecError::Overflow)?)?;
    let scaled_total = total
        .mantissa
        .checked_mul(factor)
        .ok_or(DecError::Overflow)?;
    let abs_qty = qty.mantissa.checked_abs().ok_or(DecError::Overflow)?;
    Ok(Dec::new(
        div_round_half_even(scaled_total, abs_qty)?,
        places,
    ))
}

/// The exact multiplicative reciprocal of `unit` as a terminating `Dec`, or
/// `None` when `1/unit` does not terminate in base 10 (its reduced denominator
/// has a prime factor other than 2 or 5) or when `unit` is zero.
///
/// The exact arm of [`reverse_rate`], and what [`infer_market_prices`] uses to
/// emit a reverse `P` directive alongside a cost-inferred one, so a commodity
/// that appears only as a cost DENOMINATOR (e.g. the GLD gift's
/// `… @ 0.005 GLD` leg) is priced in the directive list itself and not only in
/// the valuation graph.
fn exact_reciprocal(unit: Dec) -> Option<Dec> {
    if unit.mantissa == 0 {
        return None;
    }
    let sign = unit.mantissa.signum();
    let magnitude = unit.mantissa.checked_abs()?;
    // 1/unit = 10^p / |m|; grow the decimal places `k` (from 0) until |m| divides
    // 10^(p+k) — i.e. the reciprocal terminates. `checked_pow` returning `None`
    // (10^exp overflowing i128) means it never will → non-terminating.
    let mut exp = unit.places;
    loop {
        let numerator = 10i128.checked_pow(exp)?;
        if numerator % magnitude == 0 {
            return Some(Dec::new(sign * (numerator / magnitude), exp - unit.places));
        }
        exp = exp.checked_add(1)?;
    }
}

/// Market-price directives INFERRED from `@`/`@@` cost annotations, mirroring
/// hledger's `--infer-market-prices`. For each posting amount carrying a cost,
/// infer `P <txn.date> <amount.commodity> <unit cost>` (a `@@` total is divided
/// by `|qty|` to a per-unit price). When the unit cost's reciprocal terminates,
/// the reverse directive `P <txn.date> <cost.commodity> <1/unit cost>` is
/// inferred too — matching hledger's valuation-time price-graph reversal, so a
/// commodity seen only as a cost DENOMINATOR (the GLD gift's
/// `equity … @ 0.005 GLD`) is still valued.
///
/// The result is in journal order (date asc, then txn index). Callers append the
/// explicit `P` directives AFTER these so an explicit price wins a same-date tie
/// (hledger's precedence).
///
/// # Errors
/// Returns [`DecError`] on decimal overflow (never for realistic journals).
pub fn infer_market_prices(txns: &[Transaction]) -> Result<Vec<PriceDirective>, DecError> {
    let mut ordered: Vec<&Transaction> = txns.iter().collect();
    ordered.sort_by(|a, b| a.date.cmp(&b.date).then_with(|| a.index.0.cmp(&b.index.0)));

    let mut inferred: Vec<PriceDirective> = Vec::new();
    for txn in ordered {
        for posting in &txn.postings {
            for amount in &posting.amounts {
                let Some(cost) = amount.cost.as_deref() else {
                    continue;
                };
                if amount.quantity.is_zero() {
                    continue;
                }
                let unit = match cost.kind {
                    CostKind::Unit => cost.amount.quantity,
                    CostKind::Total => per_unit_from_total(cost.amount.quantity, amount.quantity)?,
                };
                // Forward: the posting's commodity priced in the cost commodity.
                inferred.push(PriceDirective {
                    date: txn.date.clone(),
                    commodity: amount.commodity.clone(),
                    price: Amount {
                        commodity: cost.amount.commodity.clone(),
                        quantity: unit,
                        style: cost.amount.style.clone(),
                        cost: None,
                    },
                    source_file: txn.source_file.clone(),
                });
                // Reverse (only when 1/unit terminates): lets a commodity that
                // appears solely as a cost denominator still be valued.
                if let Some(reciprocal) = exact_reciprocal(unit) {
                    inferred.push(PriceDirective {
                        date: txn.date.clone(),
                        commodity: cost.amount.commodity.clone(),
                        price: Amount {
                            commodity: amount.commodity.clone(),
                            quantity: reciprocal,
                            style: amount.style.clone(),
                            cost: None,
                        },
                        source_file: txn.source_file.clone(),
                    });
                }
            }
        }
    }
    Ok(inferred)
}

/// Value a [`MixedAmount`] in `target` at `as_of`: identity for `target` itself,
/// exact `mul_raw` by the [`PriceGraph`] rate otherwise. Commodities the target
/// is unreachable from are SKIPPED and, when `meta` is given, recorded there
/// (deduped).
///
/// # Errors
/// Returns [`DecError`] on decimal overflow.
pub fn value_at(
    ma: &MixedAmount,
    target: &Commodity,
    db: &PriceDb,
    as_of: &str,
    mut meta: Option<&mut ValuationMeta>,
) -> Result<Dec, DecError> {
    // The graph is built lazily, and only for commodities that actually need a
    // CHAIN. A direct price is a one-hop chain and one hop always wins, so
    // `PriceGraph::rate` would return exactly that price back (see
    // `a_direct_price_is_returned_unchanged`) — while building the graph costs a
    // pass over every directive. Keeping the direct hit on the old cheap path
    // leaves the common "everything is priced in the target" journal as fast as
    // it was: 5k `P` directives over 200 accounts × 12 buckets stays ~6ms
    // instead of ~215ms.
    let mut graph: Option<PriceGraph<'_>> = None;
    let mut total = Dec::zero();
    for (commodity, qty) in ma.iter() {
        if commodity == target {
            total = total.add(*qty)?;
            continue;
        }
        let rate = match db.lookup_in(commodity, target, as_of) {
            Some(price) => Some(price.quantity),
            None => graph
                .get_or_insert_with(|| db.graph_at(as_of))
                .rate(commodity, target)?,
        };
        match rate {
            Some(rate) => {
                total = total.add(mul_raw(*qty, rate)?)?;
            }
            None => {
                if let Some(sink) = meta.as_deref_mut()
                    && !sink.unpriced.contains(commodity)
                {
                    sink.unpriced.push(commodity.clone());
                }
            }
        }
    }
    Ok(total)
}

/// How many of `held` a valuation into `target` at `as_of` actually converts —
/// the numerator of the grouped reports' `valueIn` admission tests
/// (`balance_sheet::prices_any_on_sheet`,
/// `income_statement::prices_any_on_statement`). One unit of each commodity is
/// pushed through [`value_at`] itself — the same routes, at the same date — so
/// the count can never disagree with the valuation it admits.
///
/// # Errors
/// Returns [`DecError`] on decimal overflow.
pub(super) fn priced_count(
    held: &BTreeSet<Commodity>,
    target: &Commodity,
    db: &PriceDb,
    as_of: &str,
) -> Result<usize, DecError> {
    let units = held
        .iter()
        .try_fold(MixedAmount::new(), |mut units, commodity| {
            units.accumulate(commodity, Dec::new(1, 0))?;
            Ok::<_, DecError>(units)
        })?;
    let mut meta = ValuationMeta::default();
    value_at(&units, target, db, as_of, Some(&mut meta))?;
    Ok(held.len() - meta.unpriced.len())
}

#[cfg(test)]
mod tests {
    use super::super::test_support::{amount, price, txn, usd};
    use super::*;
    use crate::model::Cost;

    /// An amount carrying a per-unit (`@`) cost, for cost-inference tests.
    fn unit_cost(
        commodity: &str,
        mantissa: i128,
        places: u32,
        cost_commodity: &str,
        cost_mantissa: i128,
        cost_places: u32,
    ) -> Amount {
        let mut a = amount(commodity, mantissa, places);
        a.cost = Some(Box::new(Cost {
            kind: CostKind::Unit,
            amount: amount(cost_commodity, cost_mantissa, cost_places),
        }));
        a
    }

    fn directives() -> Vec<PriceDirective> {
        vec![
            price("2024-09-30", "EUR", amount("$", 111, 2)),
            price("2024-09-30", "AAPL", amount("$", 22800, 2)),
            price("2025-12-31", "EUR", amount("$", 110, 2)),
            price("2025-12-31", "AAPL", amount("$", 25500, 2)),
            price("2026-06-30", "EUR", amount("$", 116, 2)),
            // later same-commodity directive in a different target
            price("2026-06-30", "EUR", amount("GBP", 85, 2)),
        ]
    }

    fn c(s: &str) -> Commodity {
        Commodity(s.into())
    }

    #[test]
    fn lookup_returns_latest_le_asof_inclusive() {
        let db = PriceDb::build(&directives());
        assert_eq!(
            db.lookup(&c("AAPL"), "2025-12-30").unwrap().quantity,
            Dec::new(22800, 2)
        );
        assert_eq!(
            db.lookup(&c("AAPL"), "2025-12-31").unwrap().quantity,
            Dec::new(25500, 2)
        );
        assert_eq!(
            db.lookup(&c("AAPL"), "2026-07-08").unwrap().quantity,
            Dec::new(25500, 2)
        );
    }

    #[test]
    fn lookup_returns_none_before_first_or_unknown() {
        let db = PriceDb::build(&directives());
        assert!(db.lookup(&c("AAPL"), "2024-09-29").is_none());
        assert!(db.lookup(&c("DOGE"), "2026-07-08").is_none());
    }

    #[test]
    fn lookup_same_date_last_declared_wins() {
        let db = PriceDb::build(&directives());
        assert_eq!(
            db.lookup(&c("EUR"), "2026-06-30").unwrap().commodity,
            c("GBP")
        );
    }

    /// [`PriceDb::latest`] binary-searches for the in-effect prefix instead of
    /// filtering on the date while scanning backwards (PERF-5d). Pin it against
    /// the definition it replaced, at EVERY date in and around a series with
    /// duplicate dates and gaps — the boundaries are where an off-by-one would
    /// hide.
    #[test]
    fn latest_agrees_with_a_reverse_scan_at_every_date() {
        let series: Vec<PriceDirective> = [
            ("2024-01-10", 100),
            ("2024-03-05", 200),
            ("2024-03-05", 300), // same date twice: the LAST one wins
            ("2024-07-01", 400),
            ("2025-01-01", 500),
        ]
        .iter()
        .map(|(date, mantissa)| price(date, "AAPL", amount("$", *mantissa, 2)))
        .collect();
        let db = PriceDb::build(&series);

        for as_of in [
            "2023-12-31",
            "2024-01-09",
            "2024-01-10",
            "2024-01-11",
            "2024-03-04",
            "2024-03-05",
            "2024-03-06",
            "2024-06-30",
            "2024-07-01",
            "2024-12-31",
            "2025-01-01",
            "2025-01-02",
            "2099-01-01",
        ] {
            let want = series
                .iter()
                .rev()
                .find(|directive| directive.date.as_str() <= as_of)
                .map(|directive| directive.price.quantity);
            assert_eq!(
                db.lookup(&c("AAPL"), as_of).map(|price| price.quantity),
                want,
                "as_of {as_of}"
            );
        }
    }

    /// The same prefix drives `graph_at`, so a chain must see exactly the edges
    /// in effect — no more, no fewer — at a date sitting between directives.
    #[test]
    fn in_effect_bounds_the_price_graph_at_a_mid_series_date() {
        let db = PriceDb::build(&[
            price("2024-01-01", "GBP", amount("EUR", 120, 2)),
            price("2024-01-01", "EUR", amount("$", 110, 2)),
            price("2026-01-01", "EUR", amount("$", 200, 2)),
        ]);
        assert_eq!(rate(&db, "GBP", "$", "2025-06-30"), Some(Dec::new(132, 2)));
        assert_eq!(rate(&db, "GBP", "$", "2026-01-01"), Some(Dec::new(240, 2)));
        assert_eq!(rate(&db, "GBP", "$", "2023-12-31"), None);
    }

    #[test]
    fn lookup_in_skips_other_targets() {
        let db = PriceDb::build(&directives());
        assert_eq!(
            db.lookup_in(&c("EUR"), &c("$"), "2026-06-30")
                .unwrap()
                .quantity,
            Dec::new(116, 2)
        );
        assert_eq!(
            db.lookup_in(&c("EUR"), &c("GBP"), "2026-06-30")
                .unwrap()
                .quantity,
            Dec::new(85, 2)
        );
        assert!(db.lookup_in(&c("EUR"), &c("GBP"), "2026-06-29").is_none());
        assert!(db.lookup_in(&c("AAPL"), &c("GBP"), "2026-07-08").is_none());
    }

    #[test]
    fn base_commodity_most_frequent_then_lexical() {
        assert_eq!(
            PriceDb::build(&directives()).base_commodity(),
            Some(&c("$"))
        );
        let tie = vec![
            price("2026-01-01", "EUR", amount("GBP", 85, 2)),
            price("2026-01-02", "AAPL", amount("$", 25500, 2)),
        ];
        assert_eq!(PriceDb::build(&tie).base_commodity(), Some(&c("$")));
        assert_eq!(PriceDb::build(&[]).base_commodity(), None);
    }

    #[test]
    fn value_at_converts_and_passes_target_through() {
        let db = PriceDb::build(&directives());
        let mut ma = MixedAmount::new();
        ma.accumulate(&c("$"), Dec::new(1000, 2)).unwrap();
        ma.accumulate(&c("EUR"), Dec::new(20000, 2)).unwrap();
        // 10.00 + 200 EUR × $1.10 = 10.00 + 220.0000 = $230, kept at scale 4.
        assert_eq!(
            value_at(&ma, &c("$"), &db, "2026-01-15", None).unwrap(),
            Dec::new(2300000, 4)
        );
    }

    #[test]
    fn value_at_skips_unpriced_and_dedupes_meta() {
        let db = PriceDb::build(&directives());
        let mut ma = MixedAmount::new();
        ma.accumulate(&c("DOGE"), Dec::new(5, 0)).unwrap();
        ma.accumulate(&c("EUR"), Dec::new(10000, 2)).unwrap();
        ma.accumulate(&c("AAPL"), Dec::new(10, 0)).unwrap(); // priced in $ but asOf predates all directives
        let mut meta = ValuationMeta::default();
        assert_eq!(
            value_at(&ma, &c("$"), &db, "2024-01-01", Some(&mut meta)).unwrap(),
            Dec::new(0, 0)
        );
        // Encounter order is lexical (BTreeMap), unlike the TS insertion-ordered
        // Map; the set is identical and the report layer sorts anyway.
        assert_eq!(meta.unpriced, vec![c("AAPL"), c("DOGE"), c("EUR")]);
        // Second pass does not duplicate.
        assert_eq!(
            value_at(&ma, &c("$"), &db, "2024-01-01", Some(&mut meta)).unwrap(),
            Dec::new(0, 0)
        );
        assert_eq!(meta.unpriced, vec![c("AAPL"), c("DOGE"), c("EUR")]);
    }

    #[test]
    fn value_at_without_meta() {
        let db = PriceDb::build(&directives());
        let mut ma = MixedAmount::new();
        ma.accumulate(&c("DOGE"), Dec::new(5, 0)).unwrap();
        assert_eq!(
            value_at(&ma, &c("$"), &db, "2026-07-08", None).unwrap(),
            Dec::new(0, 0)
        );
    }

    /// The rate one unit of `from` fetches in `to`, through the graph at
    /// `as_of` — the unit under test for every chain case below.
    fn rate(db: &PriceDb, from: &str, to: &str, as_of: &str) -> Option<Dec> {
        db.graph_at(as_of)
            .rate(&c(from), &c(to))
            .expect("rate math does not overflow")
    }

    /// `P GBP 1.20 EUR` + `P EUR $1.10` prices GBP in `$` at 1.32 even though no
    /// directive mentions the pair (RPT-3).
    #[test]
    fn rate_chains_through_an_intermediate_commodity() {
        let db = PriceDb::build(&[
            price("2026-01-01", "GBP", amount("EUR", 120, 2)),
            price("2026-01-02", "EUR", amount("$", 110, 2)),
        ]);
        // hledger: `bal --value=end,'$'` on 100.00 GBP => $132.00.
        assert_eq!(rate(&db, "GBP", "$", "2026-02-01"), Some(Dec::new(132, 2)));
        let ma = MixedAmount::single(c("GBP"), Dec::new(10000, 2));
        assert_eq!(
            value_at(&ma, &c("$"), &db, "2026-02-01", None).unwrap(),
            Dec::new(1_320_000, 4)
        );
    }

    /// Three hops compose, and the chain rate keeps hledger's scale rule
    /// (normalized product, padded back to the widest edge scale).
    #[test]
    fn rate_chains_three_hops() {
        let db = PriceDb::build(&[
            price("2026-01-01", "A", amount("B", 2, 0)),
            price("2026-01-01", "B", amount("C", 3, 0)),
            price("2026-01-01", "C", amount("D", 5, 0)),
        ]);
        // hledger: 7 A --value=end,D => 210 D.
        assert_eq!(rate(&db, "A", "D", "2026-02-01"), Some(Dec::new(30, 0)));
        let ma = MixedAmount::single(c("A"), Dec::new(7, 0));
        assert_eq!(
            value_at(&ma, &c("D"), &db, "2026-02-01", None).unwrap(),
            Dec::new(210, 0)
        );
    }

    /// Only `A→B` and `C→B` exist, so reaching C needs the REVERSED `B→C` edge.
    #[test]
    fn rate_uses_a_reverse_edge_when_no_forward_chain_exists() {
        let db = PriceDb::build(&[
            price("2026-01-01", "A", amount("B", 2, 0)),
            price("2026-01-01", "C", amount("B", 4, 0)),
        ]);
        // hledger: chain A>B 2, B>C 0.25 => 10 A --value=end,C => 5 C.
        assert_eq!(rate(&db, "A", "C", "2026-02-01"), Some(Dec::new(50, 2)));
    }

    /// A forward chain of ANY length beats a one-hop reverse: `A→B→C` (6) is
    /// taken over `1/(C→A)` (0.2).
    #[test]
    fn rate_prefers_a_forward_chain_over_a_reverse_edge() {
        let db = PriceDb::build(&[
            price("2026-01-01", "A", amount("B", 2, 0)),
            price("2026-01-01", "B", amount("C", 3, 0)),
            price("2026-01-01", "C", amount("A", 5, 0)),
        ]);
        // hledger: 1 A --value=end,C => 6 C (not 0.2).
        assert_eq!(rate(&db, "A", "C", "2026-02-01"), Some(Dec::new(6, 0)));
    }

    /// Equal-length forward chains resolve by edge order — `(from, to)`
    /// ascending — NOT by the order the directives were declared.
    #[test]
    fn equal_length_chains_break_the_tie_by_commodity_order() {
        // X>M>Z = 2×5 = 10 ; X>N>Z = 3×7 = 21. hledger picks 10 either way.
        let m_first = vec![
            price("2026-01-01", "X", amount("M", 2, 0)),
            price("2026-01-01", "X", amount("N", 3, 0)),
            price("2026-01-01", "M", amount("Z", 5, 0)),
            price("2026-01-01", "N", amount("Z", 7, 0)),
        ];
        let n_first: Vec<PriceDirective> = m_first.iter().rev().cloned().collect();
        for directives in [m_first, n_first] {
            let db = PriceDb::build(&directives);
            assert_eq!(rate(&db, "X", "Z", "2026-02-01"), Some(Dec::new(10, 0)));
        }
    }

    /// The same tie-break holds for reverse edges: they keep the order of the
    /// forward edges they came from.
    #[test]
    fn equal_length_reverse_chains_break_the_tie_the_same_way() {
        let db = PriceDb::build(&[
            price("2026-01-01", "N", amount("A", 4, 0)),
            price("2026-01-01", "N", amount("Z", 7, 0)),
            price("2026-01-01", "M", amount("A", 2, 0)),
            price("2026-01-01", "M", amount("Z", 5, 0)),
        ]);
        // hledger: A>M 0.5, M>Z 5 => 2.5 (not A>N>Z = 0.25×7 = 1.75).
        assert_eq!(rate(&db, "A", "Z", "2026-02-01"), Some(Dec::new(25, 1)));
    }

    /// Every forward edge is tried before every reverse edge at the same depth,
    /// so `A>F>Z` (a forward hop then a reverse hop) wins over `A>R>Z` (a
    /// reverse hop then a forward hop).
    #[test]
    fn forward_edges_are_extended_before_reverse_edges() {
        let db = PriceDb::build(&[
            price("2026-01-01", "A", amount("F", 2, 0)),
            price("2026-01-01", "Z", amount("F", 5, 0)),
            price("2026-01-01", "R", amount("A", 3, 0)),
            price("2026-01-01", "R", amount("Z", 7, 0)),
        ]);
        // hledger: A>F 2, F>Z 0.2 => 0.4 (not A>R 1/3, R>Z 7 => 2.333…).
        assert_eq!(rate(&db, "A", "Z", "2026-02-01"), Some(Dec::new(4, 1)));
    }

    /// A cycle in the price graph terminates instead of looping, and an
    /// unreachable target is still reported as unpriced.
    #[test]
    fn a_cycle_terminates_and_an_unreachable_target_is_unpriced() {
        let db = PriceDb::build(&[
            price("2026-01-01", "A", amount("B", 2, 0)),
            price("2026-01-01", "B", amount("C", 3, 0)),
            price("2026-01-01", "C", amount("A", 5, 0)),
        ]);
        assert_eq!(rate(&db, "A", "ZZZ", "2026-02-01"), None);
        let ma = MixedAmount::single(c("A"), Dec::new(1, 0));
        let mut meta = ValuationMeta::default();
        assert_eq!(
            value_at(&ma, &c("ZZZ"), &db, "2026-02-01", Some(&mut meta)).unwrap(),
            Dec::zero()
        );
        assert_eq!(meta.unpriced, vec![c("A")]);
    }

    /// A chain is only built from prices already in effect: a directive dated
    /// after `as_of` cannot complete one.
    #[test]
    fn a_chain_needs_every_hop_in_effect_at_as_of() {
        let db = PriceDb::build(&[
            price("2026-01-01", "GBP", amount("EUR", 120, 2)),
            price("2026-03-01", "EUR", amount("$", 110, 2)),
        ]);
        assert_eq!(rate(&db, "GBP", "$", "2026-02-01"), None);
        assert_eq!(rate(&db, "GBP", "$", "2026-03-01"), Some(Dec::new(132, 2)));
    }

    /// A direct price is a one-hop chain, so it must come back bit-for-bit —
    /// same mantissa AND same scale as the directive (goldens depend on it).
    #[test]
    fn a_direct_price_is_returned_unchanged() {
        let db = PriceDb::build(&directives());
        let direct = rate(&db, "EUR", "$", "2026-01-15").unwrap();
        assert_eq!(direct.mantissa, 110);
        assert_eq!(direct.places, 2);
    }

    #[test]
    fn reverse_rate_is_exact_when_it_terminates_and_rounded_otherwise() {
        assert_eq!(reverse_rate(Dec::new(5, 3)), Some(Dec::new(200, 0))); // 1/0.005
        assert_eq!(reverse_rate(Dec::new(4, 0)), Some(Dec::new(25, 2))); // 1/4
        // 1/220 has a prime factor of 11 — hledger carries it to `Data.Decimal`'s
        // 255-place ceiling, we round half-even at MAX_RATE_PLACES.
        assert_eq!(
            reverse_rate(Dec::new(22000, 2)),
            Some(Dec::new(45_454_545, 10))
        );
        assert_eq!(
            reverse_rate(Dec::new(3, 0)),
            Some(Dec::new(3_333_333_333, 10))
        );
        assert_eq!(reverse_rate(Dec::new(-2, 0)), Some(Dec::new(-5, 1)));
        // hledger's `marketPriceReverse` maps a zero rate to a zero rate.
        assert_eq!(reverse_rate(Dec::zero()), Some(Dec::zero()));
    }

    /// The MEDIUM half of RPT-3: `10 AAPL @ $220.00` must value the `-$2,200.00`
    /// cash leg back into AAPL instead of dropping it.
    #[test]
    fn a_non_terminating_reciprocal_still_values_the_cash_leg() {
        let txns = vec![txn(
            1,
            "2026-01-05",
            vec![
                (
                    "assets:broker",
                    vec![unit_cost("AAPL", 10, 0, "$", 22000, 2)],
                ),
                ("assets:cash", vec![usd(-220_000)]),
            ],
        )];
        let db = PriceDb::build(&infer_market_prices(&txns).unwrap());
        let mut ma = MixedAmount::single(c("AAPL"), Dec::new(10, 0));
        ma.accumulate(&c("$"), Dec::new(-220_000, 2)).unwrap();
        let mut meta = ValuationMeta::default();
        let value = value_at(&ma, &c("AAPL"), &db, "2026-02-01", Some(&mut meta)).unwrap();
        // hledger nets these to 0 (its 255-place 1/220 leaves ~1e-250); ours
        // leaves the MAX_RATE_PLACES residue, well under any display precision.
        assert!(meta.unpriced.is_empty());
        assert!(value.abs().unwrap() < Dec::new(1, 6));
    }

    #[test]
    fn exact_reciprocal_terminating_and_not() {
        assert_eq!(exact_reciprocal(Dec::new(5, 3)), Some(Dec::new(200, 0))); // 1/0.005
        assert_eq!(exact_reciprocal(Dec::new(2, 0)), Some(Dec::new(5, 1))); // 1/2 = 0.5
        assert_eq!(exact_reciprocal(Dec::new(4, 0)), Some(Dec::new(25, 2))); // 1/4 = 0.25
        assert_eq!(exact_reciprocal(Dec::new(8, 0)), Some(Dec::new(125, 3))); // 1/8 = 0.125
        assert_eq!(exact_reciprocal(Dec::new(3, 0)), None); // 1/3 never terminates
        assert_eq!(exact_reciprocal(Dec::new(22000, 2)), None); // 1/220 (factor 11)
        assert_eq!(exact_reciprocal(Dec::zero()), None);
    }

    fn gld_gift() -> Vec<crate::model::Transaction> {
        // The fixture's GLD gift: the GLD lot has no cost, the equity leg prices
        // $ in GLD (`$-1,000.00 @ 0.005 GLD`).
        vec![txn(
            2,
            "2025-08-20",
            vec![
                ("assets:broker:gld", vec![amount("GLD", 5, 0)]),
                (
                    "equity:transfers",
                    vec![unit_cost("$", -100_000, 2, "GLD", 5, 3)],
                ),
            ],
        )]
    }

    #[test]
    fn infers_forward_and_reverse_from_costs() {
        let mut txns = vec![txn(
            1,
            "2024-09-16",
            vec![
                (
                    "assets:broker",
                    vec![unit_cost("AAPL", 10, 0, "$", 22000, 2)],
                ),
                ("assets:cash", vec![usd(-220_000)]),
            ],
        )];
        txns.extend(gld_gift());
        let inferred = infer_market_prices(&txns).unwrap();

        // AAPL forward (1/220 does not terminate → no reverse), then the GLD
        // gift's $→GLD forward and its GLD→$ reverse. Journal order.
        assert_eq!(inferred.len(), 3);
        assert_eq!(inferred[0].commodity, c("AAPL"));
        assert_eq!(inferred[0].date, "2024-09-16");
        assert_eq!(inferred[0].price.commodity, c("$"));
        assert_eq!(inferred[0].price.quantity, Dec::new(22000, 2));
        assert_eq!(inferred[1].commodity, c("$"));
        assert_eq!(inferred[1].price.commodity, c("GLD"));
        assert_eq!(inferred[1].price.quantity, Dec::new(5, 3));
        assert_eq!(inferred[2].commodity, c("GLD"));
        assert_eq!(inferred[2].price.commodity, c("$"));
        assert_eq!(inferred[2].price.quantity, Dec::new(200, 0));
    }

    #[test]
    fn inferred_reverse_values_a_cost_denominator_commodity() {
        let db = PriceDb::build(&infer_market_prices(&gld_gift()).unwrap());
        let ma = MixedAmount::single(c("GLD"), Dec::new(5, 0));
        // 5 GLD × $200 (= 1/0.005) = $1000, exact.
        assert_eq!(
            value_at(&ma, &c("$"), &db, "2026-01-01", None).unwrap(),
            Dec::new(1000, 0)
        );
    }
}
