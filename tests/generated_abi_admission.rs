#![cfg(feature = "wasm-sketch-host")]

use kernal_api::wasm::{SketchCompiler, SketchCompilerConfig, SketchModulePolicy};
use wasmparser::ValType;

#[derive(Clone)]
struct GeneratedImport {
    name: &'static str,
    params: &'static [ValType],
    results: &'static [ValType],
}
mod descriptor {
    use super::{GeneratedImport, ValType};
    include!("../abi/generated/admission.rs");
}
fn leb(mut n: u32, o: &mut Vec<u8>) {
    loop {
        let mut b = (n & 127) as u8;
        n >>= 7;
        if n != 0 {
            b |= 128
        }
        o.push(b);
        if n == 0 {
            return;
        }
    }
}
fn text(s: &str, o: &mut Vec<u8>) {
    leb(s.len() as u32, o);
    o.extend(s.as_bytes())
}
fn section(id: u8, b: Vec<u8>, o: &mut Vec<u8>) {
    o.push(id);
    leb(b.len() as u32, o);
    o.extend(b)
}
fn custom(n: &str, d: &[u8], o: &mut Vec<u8>) {
    let mut b = Vec::new();
    text(n, &mut b);
    b.extend(d);
    section(0, b, o)
}
fn ty(v: ValType) -> u8 {
    match v {
        ValType::I32 => 127,
        ValType::I64 => 126,
        ValType::F32 => 125,
        ValType::F64 => 124,
        _ => panic!(),
    }
}
fn wasm(imports: Vec<GeneratedImport>, meta: Option<&[u8]>) -> Vec<u8> {
    wasm_in_namespace(imports, meta, descriptor::ABI_NAMESPACE)
}
fn wasm_in_namespace(
    imports: Vec<GeneratedImport>,
    meta: Option<&[u8]>,
    namespace: &str,
) -> Vec<u8> {
    let mut w = b"\0asm\x01\0\0\0".to_vec();
    let mut t = Vec::new();
    leb((imports.len() + 8) as u32, &mut t);
    for i in &imports {
        t.push(96);
        leb(i.params.len() as u32, &mut t);
        for x in i.params {
            t.push(ty(*x))
        }
        leb(i.results.len() as u32, &mut t);
        for x in i.results {
            t.push(ty(*x))
        }
    }
    for (p, r) in [
        (&[][..], &[][..]),
        (&[][..], &[127][..]),
        (&[127, 127][..], &[][..]),
        (&[127][..], &[127][..]),
    ] {
        t.push(96);
        leb(p.len() as u32, &mut t);
        t.extend(p);
        leb(r.len() as u32, &mut t);
        t.extend(r)
    }
    // extra compatibility types: clock, two-i32, fd_write, proc_exit.
    for (p, r) in [
        (&[127, 126, 127][..], &[127][..]),
        (&[127, 127][..], &[127][..]),
        (&[127, 127, 127, 127][..], &[127][..]),
        (&[127][..], &[][..]),
    ] {
        t.push(96);
        leb(p.len() as u32, &mut t);
        t.extend(p);
        leb(r.len() as u32, &mut t);
        t.extend(r);
    }
    section(1, t, &mut w);
    let mut im = Vec::new();
    leb((imports.len() + 8) as u32, &mut im);
    text("env", &mut im);
    text("memory", &mut im);
    im.extend([2, 3, 17]);
    leb(16384, &mut im);
    for (idx, i) in imports.iter().enumerate() {
        text(namespace, &mut im);
        text(i.name, &mut im);
        im.push(0);
        leb(idx as u32, &mut im)
    }
    for (n, k) in [
        ("thread-spawn", imports.len() as u32 + 3),
        ("clock_time_get", imports.len() as u32 + 4),
        ("environ_get", imports.len() as u32 + 5),
        ("environ_sizes_get", imports.len() as u32 + 5),
        ("fd_write", imports.len() as u32 + 6),
        ("proc_exit", imports.len() as u32 + 7),
        ("sched_yield", imports.len() as u32 + 1),
    ] {
        text(
            if n == "thread-spawn" {
                "wasi"
            } else {
                "wasi_snapshot_preview1"
            },
            &mut im,
        );
        text(n, &mut im);
        im.push(0);
        leb(k, &mut im)
    }
    section(2, im, &mut w);
    let n = imports.len() as u32;
    let mut f = Vec::new();
    leb(5, &mut f);
    for x in [n, n + 1, n + 2, n + 1, n] {
        leb(x, &mut f)
    }
    section(3, f, &mut w);
    let mut e = Vec::new();
    leb(5, &mut e);
    text("memory", &mut e);
    e.extend([2, 0]);
    for (name, index) in [
        ("_start", n + 7),
        ("__main_void", n + 8),
        ("wasi_thread_start", n + 9),
        ("kernal-api-run", n + 10),
    ] {
        text(name, &mut e);
        e.push(0);
        leb(index, &mut e)
    }
    section(7, e, &mut w);
    let mut start = Vec::new();
    leb(n + 11, &mut start);
    section(8, start, &mut w);
    let mut c = Vec::new();
    leb(5, &mut c);
    for body in [
        vec![0, 11],
        vec![0, 65, 0, 11],
        vec![0, 11],
        vec![0, 65, 0, 11],
        vec![0, 11],
    ] {
        leb(body.len() as u32, &mut c);
        c.extend(body)
    }
    section(10, c, &mut w);
    if let Some(meta) = meta {
        custom("kernal-api.core-abi", meta, &mut w);
    }
    let mut features = Vec::new();
    let names = [
        "atomics",
        "bulk-memory",
        "bulk-memory-opt",
        "call-indirect-overlong",
        "extended-const",
        "multivalue",
        "mutable-globals",
        "nontrapping-fptoint",
        "reference-types",
        "sign-ext",
    ];
    leb(names.len() as u32, &mut features);
    for name in names {
        features.push(b'+');
        text(name, &mut features)
    }
    custom("target_features", &features, &mut w);
    w
}

#[path = "support/threaded_fixture.rs"]
mod threaded_fixture;

#[test]
fn generated_metadata_and_import_mutations_reject_before_compile() {
    let c = SketchCompiler::new(SketchCompilerConfig::default()).unwrap();
    let p = SketchModulePolicy::threaded_rust_v1(1 << 20, 16384).unwrap();
    let mut i: Vec<_> = descriptor::GENERATED_IMPORTS
        .iter()
        .map(|x| GeneratedImport {
            name: x.name,
            params: x.params,
            results: x.results,
        })
        .collect();
    let b = wasm(
        i.clone(),
        Some(b"fp-bindgen.core-wasm-abi@1;capabilities=0"),
    );
    c.admit(&b, p).expect("generated baseline admission");
    assert_eq!(c.compiled_module_count(), 1);
    let rejects = |bytes: Vec<u8>, expected_code: &str| {
        let Err(error) = c.admit(&bytes, p) else {
            panic!("mutation must reject")
        };
        assert_eq!(error.code(), expected_code);
        assert_eq!(c.compiled_module_count(), 1);
    };

    // A generated-looking name or namespace cannot widen the exact descriptor.
    i[0].name = "future";
    rejects(
        wasm(
            i.clone(),
            Some(b"fp-bindgen.core-wasm-abi@1;capabilities=0"),
        ),
        "forbidden-import",
    );
    let mut foreign_namespace = i.clone();
    foreign_namespace[0].name = descriptor::GENERATED_IMPORTS[0].name;
    rejects(
        wasm_in_namespace(
            foreign_namespace,
            Some(b"fp-bindgen.core-wasm-abi@1;capabilities=0"),
            "kernal-api:v9",
        ),
        "forbidden-import",
    );

    let mut wrong_signature = i.clone();
    wrong_signature[0].name = descriptor::GENERATED_IMPORTS[0].name;
    wrong_signature[0].params = &[ValType::I32];
    rejects(
        wasm(
            wrong_signature,
            Some(b"fp-bindgen.core-wasm-abi@1;capabilities=0"),
        ),
        "import-type-mismatch",
    );

    let mut missing = i.clone();
    missing[0].name = descriptor::GENERATED_IMPORTS[0].name;
    missing.pop();
    rejects(
        wasm(missing, Some(b"fp-bindgen.core-wasm-abi@1;capabilities=0")),
        "missing-required-import",
    );

    let mut duplicate_import = i.clone();
    duplicate_import[0].name = descriptor::GENERATED_IMPORTS[0].name;
    duplicate_import.push(duplicate_import[0].clone());
    rejects(
        wasm(
            duplicate_import,
            Some(b"fp-bindgen.core-wasm-abi@1;capabilities=0"),
        ),
        "forbidden-import",
    );

    let valid_imports: Vec<_> = descriptor::GENERATED_IMPORTS
        .iter()
        .map(|x| GeneratedImport {
            name: x.name,
            params: x.params,
            results: x.results,
        })
        .collect();
    rejects(
        wasm(
            valid_imports.clone(),
            Some(b"fp-bindgen.core-wasm-abi@2;capabilities=0"),
        ),
        "metadata-mismatch",
    );
    rejects(
        wasm(
            valid_imports.clone(),
            Some(b"fp-bindgen.core-wasm-abi@1;capabilities=1"),
        ),
        "metadata-mismatch",
    );
    rejects(
        wasm(valid_imports.clone(), Some(b"malformed core ABI metadata")),
        "metadata-mismatch",
    );
    rejects(
        wasm(valid_imports.clone(), Some(&[b'x'; 129])),
        "metadata-too-large",
    );
    rejects(wasm(valid_imports.clone(), None), "missing-metadata");

    let mut duplicate = wasm(
        valid_imports,
        Some(b"fp-bindgen.core-wasm-abi@1;capabilities=0"),
    );
    custom(
        "kernal-api.core-abi",
        b"fp-bindgen.core-wasm-abi@1;capabilities=0",
        &mut duplicate,
    );
    rejects(duplicate, "duplicate-metadata");

    let mut legacy = threaded_fixture::threaded_root_wasm(None, false, false, false);
    custom(
        "kernal-api.core-abi",
        b"fp-bindgen.core-wasm-abi@1;capabilities=0",
        &mut legacy,
    );
    rejects(legacy, "metadata-mismatch");
}
