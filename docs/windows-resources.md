# Windows executable resources

Enable `windows-resources` and call
`kernal_api::windows_resources::embed_windows_app_resources` from an
application's `build.rs`. It embeds the executable's icon, version information
and a Common-Controls v6 application manifest. On a non-Windows target the call
returns `Ok(())` and does nothing; outside a build script it returns
`WindowsResourceError::NotInBuildScript`.

Cargo links a build script's resources only into the binaries of the package
that runs the script, so the application, not this crate, must run it. Declare
kernal-api twice:

```toml
[dependencies]
kernal-api = { version = "=0.1.18", features = ["..."] }

[build-dependencies]
kernal-api = { version = "=0.1.18", default-features = false, features = ["windows-resources"] }
```

Keep runtime features on the `[dependencies]` line. A `kernal-api/feature`
entry in the application's `[features]` table applies to every dependency named
`kernal-api`, including the build-dependency, so it would compile those runtime
capabilities (a GUI toolkit, for example) for the host build script.

The facade privately pins embed-resource 3.0.11. The resource compiler is
`rc.exe` from the Windows SDK on MSVC hosts, `llvm-rc` when cross-compiling to
`*-pc-windows-msvc`, and the `RC`/`RC_<target>` environment variables override
both. No backend types are public.

The build-dependency still compiles this crate's non-optional native base for
the host. The feature adds only the resource compiler to it.
