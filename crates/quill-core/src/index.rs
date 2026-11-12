//! Turning raw marketplace prices into one index value.
//!
//! This is the calculation that runs inside the enclave: one median per
//! provider, then the median across providers, so a provider with five
//! hundred listings counts the same as one with twenty.

use crate::median;

/// Prices are kept only if within this range of the previous fixing: 0.4x to 2.5x.
const OUTLIER_LOW: (u128, u128) = (2, 5);
const OUTLIER_HIGH: (u128, u128) = (5, 2);

/// One provider's listings for a GPU model, in micro-USD per GPU-hour.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderListings {
    pub provider: String,
    pub prices: Vec<u128>,
}

/// One provider's contribution to an index.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderMedian {
    pub provider: String,
    pub median: u128,
    /// Listings behind the median (as reported, for providers that pre-aggregate).
    pub listings: usize,
}

/// A computed index with the evidence behind it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexResult {
    /// Median of the provider medians, micro-USD.
    pub value: u128,
    /// Providers that counted, sorted by name so the output is canonical.
    pub providers: Vec<ProviderMedian>,
}

/// Minimums an index must meet to be published.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Quorum {
    pub min_providers: usize,
    pub min_listings_per_provider: usize,
}

impl Quorum {
    /// The rule for GPU-hour indexes built from raw listings: three providers, five listings each.
    pub const GPU: Self = Self {
        min_providers: 3,
        min_listings_per_provider: 5,
    };
}

/// Why no index could be published this hour.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IndexError {
    NotEnoughProviders { have: usize, need: usize },
}

/// Whether `price` is usable: non-zero and, if a previous fixing exists, within 0.4x to 2.5x of it.
pub fn plausible(price: u128, previous: Option<u128>) -> bool {
    if price == 0 {
        return false;
    }
    match previous {
        None => true,
        Some(prev) => {
            // Multiply first for precision; if that would overflow, divide first instead.
            let scale = |(num, den): (u128, u128)| match prev.checked_mul(num) {
                Some(product) => product / den,
                None => (prev / den).saturating_mul(num),
            };
            let (low, high) = (scale(OUTLIER_LOW), scale(OUTLIER_HIGH));
            (low..=high).contains(&price)
        }
    }
}

/// Reduce one provider's raw listings to its median.
///
/// Implausible prices are dropped first. Returns `None` if fewer than
/// `min_listings` (at least one) survive.
pub fn provider_median(
    provider: &str,
    prices: &[u128],
    previous: Option<u128>,
    min_listings: usize,
) -> Option<ProviderMedian> {
    let mut kept: Vec<u128> = prices
        .iter()
        .copied()
        .filter(|p| plausible(*p, previous))
        .collect();
    if kept.len() < min_listings.max(1) {
        return None;
    }
    let listings = kept.len();
    median(&mut kept).map(|median| ProviderMedian {
        provider: provider.to_string(),
        median,
        listings,
    })
}

/// The index from one median per provider. A provider named twice counts once (the first wins).
pub fn index_from_medians(
    mut providers: Vec<ProviderMedian>,
    min_providers: usize,
) -> Result<IndexResult, IndexError> {
    let mut seen: Vec<String> = Vec::new();
    providers.retain(|p| {
        let fresh = !seen.contains(&p.provider);
        if fresh {
            seen.push(p.provider.clone());
        }
        fresh
    });
    let need = min_providers.max(1);
    if providers.len() < need {
        return Err(IndexError::NotEnoughProviders {
            have: providers.len(),
            need,
        });
    }
    providers.sort_by(|a, b| a.provider.cmp(&b.provider));
    let mut medians: Vec<u128> = providers.iter().map(|p| p.median).collect();
    let value = median(&mut medians).expect("at least one provider");
    Ok(IndexResult { value, providers })
}

/// Compute an index from every provider's raw listings.
///
/// Listings from a provider named more than once are merged, so it still gets one vote.
pub fn compute_index(
    listings: &[ProviderListings],
    previous: Option<u128>,
    quorum: Quorum,
) -> Result<IndexResult, IndexError> {
    let mut names: Vec<&str> = Vec::new();
    for source in listings {
        if !names.contains(&source.provider.as_str()) {
            names.push(&source.provider);
        }
    }
    let medians = names
        .into_iter()
        .filter_map(|name| {
            let merged: Vec<u128> = listings
                .iter()
                .filter(|l| l.provider == name)
                .flat_map(|l| l.prices.iter().copied())
                .collect();
            provider_median(name, &merged, previous, quorum.min_listings_per_provider)
        })
        .collect();
    index_from_medians(medians, quorum.min_providers)
}
