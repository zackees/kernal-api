# One-time macOS guest bootstrap

Everything else is automated. dockur/macos has no unattended install path
(confirmed against its docs), and Docker-OSX's prebuilt `:auto` tag no longer
exists on Docker Hub -- only `latest`/`master`. So the install must be driven
once by hand; after that this is fully scriptable.

## Step 1 -- install macOS (~30-60 min, one core)

Open http://localhost:8006

1. Disk Utility -> select the ~128 GB QEMU HARDDISK -> Erase -> APFS -> Erase
2. Quit Disk Utility -> "Reinstall macOS Ventura" -> Continue -> pick that disk
3. Walk the setup screens. Skip Apple ID. Create a local account.
   **Use username `runner`** (the scripts assume it; override with GUEST_USER).
   Pick any password you'll remember.

## Step 2 -- enable SSH (in the guest)

System Settings -> General -> Sharing -> **Remote Login: on**

Then confirm the guest's IP inside the guest (Terminal):

    ipconfig getifaddr en0

## Step 3 -- expose SSH to the host

dockur maps only :8006 by default. Recreate the container with a port map,
keeping the SAME storage volume so the install is preserved:

    docker rm -f kernal-macos-x86
    docker run -d --name kernal-macos-x86 \
      --device=/dev/kvm --device=/dev/net/tun --cap-add NET_ADMIN \
      -p 8006:8006 -p 2222:22 \
      -e VERSION=ventura -e RAM_SIZE=8G -e CPU_CORES=1 -e DISK_SIZE=128G \
      -v ~/.clud/docker-mac-x86/storage:/storage \
      dockurr/macos

## Step 4 -- install cargo-nextest in the guest

From the host (`cargo-nextest` here is a universal Mach-O, verified
`ca fe ba be 00 00 00 02`, so it runs on Intel):

    scp -P 2222 ~/.clud/docker-mac-x86/cargo-nextest runner@localhost:~/
    ssh -P 2222 runner@localhost 'sudo mv ~/cargo-nextest /usr/local/bin/ && sudo chmod +x /usr/local/bin/cargo-nextest'

No Rust toolchain, no Xcode CLT, no Homebrew needed -- the test binaries are
prebuilt on Linux.

## Step 5 -- snapshot, so this never has to happen again

    docker stop kernal-macos-x86
    tar -I 'zstd -T0' -cf ~/.clud/docker-mac-x86/macos-ready.tar.zst \
      -C ~/.clud/docker-mac-x86 storage
    docker start kernal-macos-x86

Restoring that tarball rebuilds a ready guest in seconds.

## Then the loop is fully automated

    ./build-archive.sh     # Linux cross-build, ~85 s, no Mac involved
    ./run-in-guest.sh      # ship + execute, real exit code propagates
