@echo off
rem Windows counterpart of the extensionless `cargo` shim beside it.
rem
rem Both files do the same thing, and both are needed: `CreateProcess` resolves a
rem bare `cargo` through PATHEXT, so an extensionless script in PATH is never
rem selected on Windows. Shipping only the script would leave this host silently
rem re-entering soldr -- the exact failure the shim was written to prevent --
rem while the job still reported green (#147).
setlocal
if "%RUSTUP_TOOLCHAIN%"=="" set "RUSTUP_TOOLCHAIN=nightly-2026-05-28"
rustup run %RUSTUP_TOOLCHAIN% cargo %*
exit /b %ERRORLEVEL%
