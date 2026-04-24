{
  description = "Reproducible Choracle Nitro Enclave image";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-25.11";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    nitridingDaemon = {
      url = "github:brave/nitriding-daemon/2b7dfefaee56819681b7f5a4ee8d66a417ad457d";
      flake = false;
    };
  };

  outputs =
    {
      self,
      nixpkgs,
      rust-overlay,
      nitridingDaemon,
    }:
    let
      lib = nixpkgs.lib;
      systems = [ "aarch64-linux" ];
      forAllSystems =
        f:
        lib.genAttrs systems (
          system:
          f (
            import nixpkgs {
              inherit system;
              overlays = [ rust-overlay.overlays.default ];
            }
          )
        );
    in
    {
      packages = forAllSystems (
        pkgs:
        let
          pname = "coinbase-candle-prover";
          version = "0.1.0";
          rustToolchain = pkgs.rust-bin.stable."1.92.0".minimal;
          rustPlatform = pkgs.makeRustPlatform {
            cargo = rustToolchain;
            rustc = rustToolchain;
          };

          source = lib.cleanSourceWith {
            src = ./.;
            filter =
              path: type:
              let
                name = baseNameOf path;
              in
              !(
                name == ".git"
                || name == "target"
                || name == "result"
                || lib.hasSuffix ".eif" name
              );
          };

          choracle = rustPlatform.buildRustPackage {
            inherit pname version;
            src = source;
            cargoLock.lockFile = ./Cargo.lock;
            cargoBuildFlags = [
              "--bin"
              "enclave-prover"
              "--bin"
              "verify-proof"
              "--bin"
              "choracle-runtime-config"
            ];
            doCheck = false;
          };

          nitriding = pkgs.buildGoModule {
            pname = "nitriding";
            version = "2b7dfefaee56819681b7f5a4ee8d66a417ad457d";
            src = nitridingDaemon;

            # Refresh with:
            #   nix build .#nitriding
            # and replace this fake hash with the hash Nix reports.
            vendorHash = "sha256-cVlPSXcn44X3Lusq1gmlPY+b0k8Vd1uKZVIwxYQbMgM=";

            subPackages = [ "." ];
            preBuild = ''
              export CGO_ENABLED=0
            '';
            GOFLAGS = [
              "-buildvcs=false"
            ];
            ldflags = [
              "-s"
              "-w"
            ];
            doCheck = false;
          };

          enclaveRoot = pkgs.runCommand "choracle-enclave-root" { } ''
            mkdir -p "$out/usr/local/bin"
            mkdir -p "$out/etc"
            install -m 0755 ${choracle}/bin/enclave-prover "$out/usr/local/bin/enclave-prover"
            install -m 0755 ${choracle}/bin/verify-proof "$out/usr/local/bin/verify-proof"
            install -m 0755 ${choracle}/bin/choracle-runtime-config "$out/usr/local/bin/choracle-runtime-config"
            install -m 0755 ${nitriding}/bin/nitriding-daemon "$out/usr/local/bin/nitriding"
            install -m 0755 ${./deploy/enclave-entrypoint.sh} "$out/usr/local/bin/enclave-entrypoint.sh"
            ln -s /run/resolvconf/resolv.conf "$out/etc/resolv.conf"
          '';

          imageRoot = pkgs.buildEnv {
            name = "choracle-enclave-image-root";
            paths = [
              enclaveRoot
              pkgs.busybox
              pkgs.cacert
            ];
            pathsToLink = [
              "/bin"
              "/etc"
              "/usr/local/bin"
            ];
          };
        in
        {
          inherit choracle nitriding;

          choracle-tools-aarch64 = choracle;

          choracle-enclave-oci-aarch64 = pkgs.dockerTools.buildLayeredImage {
            name = "choracle-enclave";
            tag = "reproducible";
            architecture = "arm64";
            created = "1970-01-01T00:00:01Z";
            contents = [ imageRoot ];
            config = {
              Entrypoint = [ "/usr/local/bin/enclave-entrypoint.sh" ];
              Env = [
                "SSL_CERT_FILE=/etc/ssl/certs/ca-bundle.crt"
                "PROOF_HTTP_LISTEN=127.0.0.1:8081"
                "CHORACLE_PARENT_CID=3"
                "CHORACLE_FQDN_CONFIG_PORT=11001"
              ];
            };
          };

          default = self.packages.${pkgs.stdenv.hostPlatform.system}.choracle-enclave-oci-aarch64;
        }
      );

      checks = forAllSystems (pkgs: {
        choracle = self.packages.${pkgs.stdenv.hostPlatform.system}.choracle;
        nitriding = self.packages.${pkgs.stdenv.hostPlatform.system}.nitriding;
        enclave-image = self.packages.${pkgs.stdenv.hostPlatform.system}.choracle-enclave-oci-aarch64;
      });
    };
}
