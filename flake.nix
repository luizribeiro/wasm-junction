{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    git-hooks = {
      url = "github:cachix/git-hooks.nix";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs =
    {
      nixpkgs,
      flake-utils,
      git-hooks,
      rust-overlay,
      ...
    }:
    flake-utils.lib.eachDefaultSystem (
      system:
      let
        overlays = [ (import rust-overlay) ];
        pkgs = import nixpkgs { inherit system overlays; };
        isLinux = pkgs.stdenv.hostPlatform.isLinux;
        playwrightBrowsers = pkgs.lib.optionals isLinux [ pkgs.playwright-driver.browsers ];
        playwrightBrowserEnv = pkgs.lib.optionalAttrs isLinux {
          PLAYWRIGHT_NIX_BROWSERS_PATH = "${pkgs.playwright-driver.browsers}";
        };
        toolchain = pkgs.rust-bin.stable."1.98.1".minimal.override {
          extensions = [
            "clippy"
            "rust-analyzer"
            "rust-src"
            "rustfmt"
          ];
          targets = [
            "wasm32-unknown-unknown"
            "wasm32-wasip2"
          ];
        };
        cargoHook =
          {
            name,
            text,
            runtimeInputs ? [ ],
            stages ? [ "pre-commit" ],
          }:
          {
            enable = true;
            entry = "${
              pkgs.writeShellApplication {
                inherit name text;
                runtimeInputs = [ toolchain ] ++ runtimeInputs;
              }
            }/bin/${name}";
            files = "(^|/)(Cargo\\.(toml|lock)|.*\\.rs)$";
            pass_filenames = false;
            inherit stages;
          };
        offlineHooks = {
          nixfmt.enable = true;
          check-toml.enable = true;
          end-of-file-fixer.enable = true;
          trim-trailing-whitespace.enable = true;
        };
        cargoHooks = {
          rustfmt = cargoHook {
            name = "rustfmt-hook";
            text = "cargo fmt --all -- --check";
          };
          clippy = cargoHook {
            name = "clippy-hook";
            text = ''
              cargo clippy --workspace --all-targets --locked -- -W clippy::pedantic -D warnings
              cargo check -p wasm-junction-core --target wasm32-unknown-unknown --locked
              cargo check -p wasm-junction --target wasm32-unknown-unknown --locked
              cargo check -p wasm-junction --test bindgen_calls --test bindgen_handles --test bindgen_resources --test bindgen_streams --test bindgen_with --target wasm32-unknown-unknown --no-default-features --locked
            '';
          };
          cargo-deny = cargoHook {
            name = "cargo-deny-hook";
            runtimeInputs = [ pkgs.cargo-deny ];
            text = "cargo deny check bans licenses sources";
          };
          cargo-nextest = cargoHook {
            name = "cargo-nextest-hook";
            runtimeInputs = [ pkgs.cargo-nextest ];
            text = ''
              cargo nextest run --workspace --locked --no-tests fail
              cargo nextest run -p wasm-junction --no-default-features --locked
            '';
            stages = [ "pre-push" ];
          };
          browser-tests = cargoHook {
            name = "browser-tests-hook";
            runtimeInputs = [
              pkgs.nodejs_24
              pkgs.playwright-driver
              pkgs.wasm-bindgen-cli
            ]
            ++ playwrightBrowsers;
            text = ''
              scripts/browser-test all -- -p wasm-junction-jco --lib
              scripts/browser-test all -- -p wasm-junction-jco --test browser
            '';
            stages = [ "pre-push" ];
          };
          doctests = cargoHook {
            name = "doctests-hook";
            text = "cargo test --doc --workspace --locked";
            stages = [ "pre-push" ];
          };
          docs = cargoHook {
            name = "docs-hook";
            text = ''
              RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --locked
            '';
            stages = [ "pre-push" ];
          };
        };
        gitHooks = git-hooks.lib.${system}.run {
          src = ./.;
          hooks = offlineHooks // cargoHooks;
        };
      in
      {
        checks.pre-commit = git-hooks.lib.${system}.run {
          src = ./.;
          hooks = offlineHooks;
        };

        devShells.default = pkgs.mkShell (
          {
            packages = [
              toolchain
              pkgs.wasm-tools
              pkgs.cargo-nextest
              pkgs.cargo-deny
              pkgs.git-absorb
              pkgs.libiconv
              pkgs.nodejs_24
              pkgs.playwright-driver
              pkgs.wasm-bindgen-cli
            ]
            ++ playwrightBrowsers
            ++ gitHooks.enabledPackages;
            PLAYWRIGHT_NODE_PATH = "${pkgs.playwright-driver}";
            inherit (gitHooks) shellHook;
          }
          // playwrightBrowserEnv
        );
      }
    );
}
