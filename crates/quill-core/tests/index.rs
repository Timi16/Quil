//! Tests for `quill_core::index`.

use quill_core::index::*;

fn provider(name: &str, prices: &[u128]) -> ProviderListings {
    ProviderListings {
        provider: name.into(),
        prices: prices.to_vec(),
    }
}

fn five(price: u128) -> Vec<u128> {
    vec![price; 5]
}

#[test]
fn median_of_provider_medians() {
    let listings = [
        provider("akash", &five(2_840_000)),
        provider("runpod", &five(2_990_000)),
        provider("vast", &five(3_120_000)),
        provider("shadeform", &five(3_210_000)),
    ];
    let result = compute_index(&listings, None, Quorum::GPU).unwrap();
    assert_eq!(result.value, 3_055_000);
    let names: Vec<_> = result
        .providers
        .iter()
        .map(|p| p.provider.as_str())
        .collect();
    assert_eq!(names, ["akash", "runpod", "shadeform", "vast"]); // canonical order
}

#[test]
fn a_flood_of_listings_from_one_provider_is_one_vote() {
    let mut spam = vec![100_000u128; 500]; // 500 fake cheap listings
    spam.extend(five(100_000));
    let listings = [
        provider("vast", &spam),
        provider("akash", &five(3_000_000)),
        provider("runpod", &five(3_100_000)),
    ];
    // Without a previous fixing nothing is filtered, yet the index barely moves.
    assert_eq!(
        compute_index(&listings, None, Quorum::GPU).unwrap().value,
        3_000_000
    );
}

#[test]
fn outliers_against_the_previous_fixing_are_dropped() {
    let listings = [
        provider(
            "akash",
            &[
                3_000_000, 3_000_000, 3_000_000, 3_000_000, 3_000_000, 1, 90_000_000,
            ],
        ),
        provider("runpod", &five(3_050_000)),
        provider("vast", &five(3_100_000)),
    ];
    let result = compute_index(&listings, Some(3_000_000), Quorum::GPU).unwrap();
    assert_eq!(result.value, 3_050_000);
    assert_eq!(result.providers[0].listings, 5); // the two junk listings were dropped
}

#[test]
fn bounds_of_the_outlier_filter_are_inclusive() {
    // 0.4x and 2.5x of 1_000_000 are kept; one micro-USD beyond either is not.
    let listings = [provider("a", &[400_000, 2_500_000, 399_999, 2_500_001])];
    let q = Quorum {
        min_providers: 1,
        min_listings_per_provider: 1,
    };
    let result = compute_index(&listings, Some(1_000_000), q).unwrap();
    assert_eq!(result.providers[0].listings, 2);
}

#[test]
fn thin_providers_do_not_count() {
    let listings = [
        provider("akash", &five(3_000_000)),
        provider("runpod", &five(3_000_000)),
        provider("vast", &[3_000_000; 4]), // one listing short
    ];
    assert_eq!(
        compute_index(&listings, None, Quorum::GPU),
        Err(IndexError::NotEnoughProviders { have: 2, need: 3 })
    );
}

#[test]
fn no_listings_at_all_is_an_error_not_a_panic() {
    assert_eq!(
        compute_index(&[], None, Quorum::GPU),
        Err(IndexError::NotEnoughProviders { have: 0, need: 3 })
    );
    let empty = [provider("a", &[]), provider("b", &[0, 0, 0, 0, 0])];
    assert!(compute_index(&empty, None, Quorum::GPU).is_err());
}

#[test]
fn a_provider_named_twice_is_merged_into_one_vote() {
    let listings = [
        provider("vast", &[1_000_000; 3]),
        provider("vast", &[1_000_000; 3]),
        provider("akash", &five(3_000_000)),
        provider("runpod", &five(3_000_000)),
    ];
    let result = compute_index(&listings, None, Quorum::GPU).unwrap();
    assert_eq!(result.providers.len(), 3);
    assert_eq!(
        result
            .providers
            .iter()
            .find(|p| p.provider == "vast")
            .unwrap()
            .listings,
        6
    );
}

#[test]
fn a_zero_quorum_still_needs_one_provider() {
    let q = Quorum {
        min_providers: 0,
        min_listings_per_provider: 0,
    };
    assert!(compute_index(&[], None, q).is_err());
    assert_eq!(
        compute_index(&[provider("a", &[7])], None, q)
            .unwrap()
            .value,
        7
    );
}

fn pm(provider: &str, median: u128, listings: usize) -> ProviderMedian {
    ProviderMedian {
        provider: provider.into(),
        median,
        listings,
    }
}

#[test]
fn index_from_pre_aggregated_medians() {
    let medians = vec![
        pm("vast", 2_667_000, 11),
        pm("akash", 2_560_000, 56),
        pm("runpod", 3_340_000, 2),
        pm("shadeform", 2_728_000, 39),
    ];
    let result = index_from_medians(medians, 3).unwrap();
    assert_eq!(result.value, 2_697_500); // mean of the two middle medians
    assert_eq!(result.providers[0].provider, "akash");
}

#[test]
fn a_repeated_provider_median_counts_once() {
    let medians = vec![pm("vast", 1, 5), pm("vast", 999, 5), pm("akash", 3, 5)];
    assert_eq!(
        index_from_medians(medians.clone(), 3),
        Err(IndexError::NotEnoughProviders { have: 2, need: 3 })
    );
    assert_eq!(index_from_medians(medians, 2).unwrap().value, 2);
}

#[test]
fn provider_median_applies_the_filter_and_the_minimum() {
    assert_eq!(
        provider_median("a", &[3, 1, 2], None, 3),
        Some(pm("a", 2, 3))
    );
    assert_eq!(provider_median("a", &[3, 1, 2], None, 4), None);
    assert_eq!(provider_median("a", &[0, 0], None, 0), None);
    // With a previous fixing of 100, only 40..=250 survives.
    assert_eq!(
        provider_median("a", &[39, 40, 100, 250, 251], Some(100), 1),
        Some(pm("a", 100, 3))
    );
}

#[test]
fn plausible_rejects_zero_and_far_off_prices() {
    assert!(!plausible(0, None));
    assert!(plausible(1, None));
    assert!(plausible(u128::MAX, Some(u128::MAX)));
    assert!(!plausible(1, Some(1_000)));
}
