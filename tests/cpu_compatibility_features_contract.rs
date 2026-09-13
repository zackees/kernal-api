use kernal_api::platform::host::cpu_compatibility_features;

#[test]
fn feature_names_remain_an_ordered_unique_subset() {
    let vocabulary = [
        "sse2", "sse4.2", "avx", "avx2", "avx512f", "fma", "bmi1", "bmi2",
    ];
    let features = cpu_compatibility_features();
    let positions: Vec<_> = features
        .iter()
        .map(|name| {
            vocabulary
                .iter()
                .position(|candidate| candidate == name)
                .expect("known feature")
        })
        .collect();
    assert!(positions.windows(2).all(|pair| pair[0] < pair[1]));
    #[cfg(not(any(target_arch = "x86", target_arch = "x86_64")))]
    assert!(features.is_empty());
}

#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
#[test]
fn canonical_feature_query_matches_native_detection() {
    let expected: Vec<_> = [
        ("sse2", std::arch::is_x86_feature_detected!("sse2")),
        ("sse4.2", std::arch::is_x86_feature_detected!("sse4.2")),
        ("avx", std::arch::is_x86_feature_detected!("avx")),
        ("avx2", std::arch::is_x86_feature_detected!("avx2")),
        ("avx512f", std::arch::is_x86_feature_detected!("avx512f")),
        ("fma", std::arch::is_x86_feature_detected!("fma")),
        ("bmi1", std::arch::is_x86_feature_detected!("bmi1")),
        ("bmi2", std::arch::is_x86_feature_detected!("bmi2")),
    ]
    .into_iter()
    .filter_map(|(name, present)| present.then_some(name))
    .collect();
    assert_eq!(cpu_compatibility_features(), expected);
}
