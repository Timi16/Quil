//! The whole program against saved real responses, including source outages.

mod common;

use std::collections::BTreeMap;

use common::{fixture, Fixtures};
use quill_oracle::{run, Input};

fn input(indexes: &[&str]) -> Input {
    Input {
        slot: 490_334,
        indexes: indexes.iter().map(|s| s.to_string()).collect(),
        previous: BTreeMap::new(),
    }
}

#[test]
fn h100_is_the_median_of_four_provider_medians() {
    let result = run(&input(&["H100"]), &Fixtures::default());
    assert_eq!(
        result.evidence,
        [
            "H100|akash|2560000|56",
            "H100|runpod|3340000|2",
            "H100|shadeform|2727500|39",
            "H100|vast|2666852|11"
        ]
    );
    // Middle two are 2.666852 and 2.727500.
    assert_eq!(result.output.v["H100"], 2_697_176);
    assert_eq!(result.output.n["H100"], 4);
    assert_eq!(result.output.slot, 490_334);
    assert!(result.problems.is_empty(), "{:?}", result.problems);
}

#[test]
fn inference_index_reads_the_pinned_model() {
    let result = run(&input(&["INF_OUT"]), &Fixtures::default());
    assert_eq!(result.output.v["INF_OUT"], 1_200_000);
    assert_eq!(result.output.n["INF_OUT"], 1);
    assert_eq!(result.evidence, ["INF_OUT|near-ai|1200000|1"]);
}

#[test]
fn one_source_down_still_publishes_with_three() {
    let result = run(
        &input(&["H100"]),
        &Fixtures {
            down: vec!["shadeform"],
            ..Default::default()
        },
    );
    assert_eq!(result.output.n["H100"], 3);
    assert_eq!(result.output.v["H100"], 2_666_852); // median of akash, vast, runpod
    assert_eq!(result.problems, ["H100: shadeform: shadeform timed out"]);
}

#[test]
fn two_sources_down_publishes_nothing_for_that_index() {
    let result = run(
        &input(&["H100", "INF_OUT"]),
        &Fixtures {
            down: vec!["shadeform", "akash"],
            ..Default::default()
        },
    );
    assert!(!result.output.v.contains_key("H100"));
    assert!(!result.output.n.contains_key("H100"));
    // An unrelated index is unaffected.
    assert_eq!(result.output.v["INF_OUT"], 1_200_000);
}

#[test]
fn an_index_with_too_few_sources_in_the_data_is_left_out() {
    // The saved Vast and Akash responses have no B200, leaving two providers.
    let result = run(&input(&["B200"]), &Fixtures::default());
    assert!(result.output.v.is_empty());
    assert!(
        result
            .problems
            .iter()
            .any(|p| p.contains("NotEnoughProviders")),
        "{:?}",
        result.problems
    );
}

#[test]
fn a_source_returning_garbage_counts_as_down() {
    let fetch = Fixtures {
        overrides: vec![("vast", "<html>captcha</html>".into())],
        ..Default::default()
    };
    let result = run(&input(&["H100"]), &fetch);
    assert_eq!(result.output.n["H100"], 3);
    assert!(result.problems[0].starts_with("H100: vast: not JSON"));
}

#[test]
fn a_marketplace_flooded_with_fake_listings_cannot_move_the_index_far() {
    // Someone posts 500 H100 offers at one cent on Vast.
    let offers: Vec<String> = (0..500)
        .map(|_| {
            r#"{"gpu_name":"H100 SXM","num_gpus":1,"dph_total":0.01,"rentable":true}"#.to_string()
        })
        .collect();
    let fetch = Fixtures {
        overrides: vec![("vast", format!(r#"{{"offers":[{}]}}"#, offers.join(",")))],
        ..Default::default()
    };

    // With no history the fake median is one vote of four: the index moves to the next provider, not to a cent.
    let cold = run(&input(&["H100"]), &fetch);
    assert_eq!(cold.output.v["H100"], 2_643_750); // mean of akash 2.56 and shadeform 2.7275

    // With a previous fixing the fake listings are filtered out entirely and Vast drops out.
    let mut warm = input(&["H100"]);
    warm.previous.insert("H100".into(), 2_697_176);
    let warm = run(&warm, &fetch);
    assert_eq!(warm.output.n["H100"], 3);
    assert_eq!(warm.output.v["H100"], 2_727_500);
}

#[test]
fn unknown_and_repeated_indexes_are_harmless() {
    let result = run(&input(&["H100", "DOGE", "H100", ""]), &Fixtures::default());
    assert_eq!(result.output.v.len(), 1);
    assert_eq!(result.evidence.len(), 4, "H100 evidence is not doubled");
}

#[test]
fn the_same_inputs_give_the_same_hash_and_different_inputs_do_not() {
    let a = run(&input(&["H100", "INF_OUT"]), &Fixtures::default());
    let b = run(&input(&["INF_OUT", "H100"]), &Fixtures::default());
    assert_eq!(
        a.output.inputs, b.output.inputs,
        "request order does not matter"
    );
    assert!(a.output.inputs.starts_with("sha256:") && a.output.inputs.len() == 71);

    let c = run(
        &input(&["H100", "INF_OUT"]),
        &Fixtures {
            down: vec!["runpod"],
            ..Default::default()
        },
    );
    assert_ne!(a.output.inputs, c.output.inputs);
}

#[test]
fn the_result_fits_outlayers_output_limit() {
    let result = run(&input(&["H100", "B200", "INF_OUT"]), &Fixtures::default());
    let printed = serde_json::to_string(&result.output).unwrap();
    assert!(printed.len() < 900, "{} bytes", printed.len());
    // And it is exactly what fixing-oracle parses.
    assert!(printed.starts_with(r#"{"slot":490334,"v":{"#), "{printed}");
}

#[test]
fn fixtures_are_real_saved_responses() {
    for name in [
        "vast_h100.json",
        "akash_gpu_prices.json",
        "runpod_gpu_types.json",
        "shadeform_instance_types.json",
        "nearai_model_list.json",
    ] {
        assert!(fixture(name).len() > 100, "{name} is empty");
    }
}
