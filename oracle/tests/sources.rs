//! Each parser against a saved real response from its source (captured 11 Nov 2026).

mod common;

use common::fixture;
use quill_oracle::sources::*;

const A100: Gpu = Gpu {
    index: "A100",
    vast_name: "A100",
    akash_model: "a100",
    runpod_id: "NVIDIA A100 80GB PCIe",
    shadeform_type: "A100",
};

#[test]
fn vast_prices_are_per_gpu() {
    let prices = parse_vast(&fixture("vast_h100.json"), &H100).unwrap();
    assert_eq!(prices.len(), 11);
    // A 2-GPU offer at $4.668/h counts as $2.334 per GPU, not $4.668.
    assert!(
        prices.iter().all(|p| (2_000_000..5_000_000).contains(p)),
        "{prices:?}"
    );
    assert_eq!(*prices.iter().min().unwrap(), 2_333_704);
}

#[test]
fn vast_ignores_other_models_and_broken_offers() {
    let body = r#"{"offers":[
        {"gpu_name":"H100 SXM","num_gpus":1,"dph_total":2.5,"rentable":true},
        {"gpu_name":"RTX 4090","num_gpus":1,"dph_total":0.4,"rentable":true},
        {"gpu_name":"H100 SXM","num_gpus":0,"dph_total":2.5,"rentable":true},
        {"gpu_name":"H100 SXM","num_gpus":1,"dph_total":-1,"rentable":true},
        {"gpu_name":"H100 SXM","num_gpus":1,"dph_total":"free","rentable":true},
        {"gpu_name":"H100 SXM","num_gpus":1,"dph_total":2.9,"rentable":false}
    ]}"#;
    assert_eq!(parse_vast(body, &H100).unwrap(), vec![2_500_000]);
}

#[test]
fn vast_url_asks_for_on_demand_rentable_offers_of_the_model() {
    let url = vast_url(&H100);
    assert!(url.starts_with("https://console.vast.ai/api/v0/bundles/?q=%7B"));
    assert!(
        url.contains("H100%20SXM")
            && url.contains("on-demand")
            && !url.contains(' ')
            && !url.contains('"')
    );
}

#[test]
fn akash_gives_its_own_median_and_gpu_count() {
    assert_eq!(
        parse_akash(&fixture("akash_gpu_prices.json"), &H100).unwrap(),
        Some((2_560_000, 56))
    );
    assert_eq!(
        parse_akash(&fixture("akash_gpu_prices.json"), &B200).unwrap(),
        None
    );
}

#[test]
fn akash_uses_the_entry_with_the_most_gpus_when_a_model_is_listed_twice() {
    // A100 appears twice in the real response: 1 GPU at $0.84 and 200 GPUs at $1.49.
    assert_eq!(
        parse_akash(&fixture("akash_gpu_prices.json"), &A100).unwrap(),
        Some((1_490_000, 200))
    );
}

#[test]
fn runpod_gives_secure_and_community_prices() {
    let mut prices = parse_runpod(&fixture("runpod_gpu_types.json"), &H100).unwrap();
    prices.sort_unstable();
    assert_eq!(prices, vec![2_690_000, 3_990_000]);
    assert_eq!(
        parse_runpod(&fixture("runpod_gpu_types.json"), &B200)
            .unwrap()
            .len(),
        2
    );
}

#[test]
fn shadeform_prices_are_cents_per_instance() {
    let prices = parse_shadeform(&fixture("shadeform_instance_types.json"), &H100).unwrap();
    assert_eq!(prices.len(), 39);
    // 8 GPUs for 392 cents an hour is $0.49 per GPU.
    let body = r#"{"instance_types":[{"gpu_type":"H100","num_gpus":8,"hourly_price":392}]}"#;
    assert_eq!(parse_shadeform(body, &H100).unwrap(), vec![490_000]);
}

#[test]
fn near_ai_price_is_per_million_output_tokens() {
    let body = fixture("nearai_model_list.json");
    // amount 1200 at scale 9 is $0.0000012 per token: $1.20 per million.
    assert_eq!(
        parse_near_ai(&body, "deepseek-ai/DeepSeek-V4.1-Flash-1017"),
        Ok(1_200_000)
    );
    assert!(parse_near_ai(&body, "nobody/no-such-model").is_err());
}

#[test]
fn near_ai_handles_other_scales_and_rejects_bad_prices() {
    let price = |amount: &str, scale: &str, currency: &str| {
        format!(
            r#"{{"models":[{{"modelId":"m","outputCostPerToken":{{"amount":{amount},"scale":{scale},"currency":"{currency}"}}}}]}}"#
        )
    };
    assert_eq!(parse_near_ai(&price("5", "6", "USD"), "m"), Ok(5_000_000)); // $5 per million
    assert_eq!(parse_near_ai(&price("12", "12", "USD"), "m"), Ok(12));
    assert_eq!(parse_near_ai(&price("12000", "15", "USD"), "m"), Ok(12));
    assert!(parse_near_ai(&price("0", "9", "USD"), "m").is_err());
    assert!(parse_near_ai(&price("1200", "9", "EUR"), "m").is_err());
    assert!(parse_near_ai(&price("1", "200", "USD"), "m").is_err());
    assert!(parse_near_ai(&price("18446744073709551615", "0", "USD"), "m").is_ok());
}

#[test]
fn every_parser_survives_garbage() {
    for body in [
        "",
        "null",
        "[]",
        "{}",
        "<html>502</html>",
        r#"{"offers":7,"models":"x","data":null,"instance_types":{}}"#,
    ] {
        assert!(parse_vast(body, &H100).is_err(), "vast accepted {body:?}");
        assert!(parse_akash(body, &H100).is_err());
        assert!(parse_runpod(body, &H100).is_err());
        assert!(parse_shadeform(body, &H100).is_err());
        assert!(parse_near_ai(body, "m").is_err());
    }
}

#[test]
fn dollar_conversion_rejects_nonsense() {
    assert_eq!(usd_to_micro(2.56), Some(2_560_000));
    assert_eq!(usd_to_micro(0.000001), Some(1));
    for bad in [0.0, -1.0, f64::NAN, f64::INFINITY, 1e12] {
        assert_eq!(usd_to_micro(bad), None);
    }
}
