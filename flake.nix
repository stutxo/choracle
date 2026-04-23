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

            postPatch = ''
              awk '{ print } index($0, "e.extPubSrv.TLSConfig = certManager.TLSConfig()") { print "    e.extPrivSrv.TLSConfig = e.extPubSrv.TLSConfig.Clone()" }' enclave.go > enclave.go.new
              mv enclave.go.new enclave.go
              awk 'seen && /e.extPrivSrv.TLSConfig = e.extPubSrv.TLSConfig.Clone()/ { ok = 1 } /e.extPubSrv.TLSConfig = certManager.TLSConfig\(\)/ { seen = 1 } END { exit ok ? 0 : 1 }' enclave.go
              awk '{ print } index($0, "e.extPubSrv.TLSConfig = certManager.TLSConfig()") { print "    go func() {"; print "        srv := &http.Server{"; print "            Addr: \":80\","; print "            Handler: certManager.HTTPHandler(nil),"; print "        }"; print "        elog.Printf(\"Starting ACME HTTP-01 Web server at :80.\")"; print "        if err := srv.ListenAndServe(); err != nil && !errors.Is(err, http.ErrServerClosed) {"; print "            elog.Printf(\"ACME HTTP-01 Web server error: %v\", err)"; print "        }"; print "    }()" }' enclave.go > enclave.go.new
              mv enclave.go.new enclave.go
              awk 'seen && /Starting ACME HTTP-01 Web server at :80/ { ok = 1 } /e.extPubSrv.TLSConfig = certManager.TLSConfig\(\)/ { seen = 1 } END { exit ok ? 0 : 1 }' enclave.go
            '';

            subPackages = [ "." ];
            preBuild = ''
              export CGO_ENABLED=0
              autocert_path="vendor/golang.org/x/crypto/acme/autocert/autocert.go"
              test -f "$autocert_path"
              chmod -R u+w "$(dirname "$autocert_path")"
              awk '
                /typ := \[\]string\{"tls-alpn-01"\}/ {
                  print "\tif m.tryHTTP01 {"
                  print "\t\treturn []string{\"http-01\"}"
                  print "\t}"
                  print "\treturn []string{\"tls-alpn-01\"}"
                  skip = 4
                  next
                }
                skip > 0 { skip--; next }
                { print }
              ' "$autocert_path" > "$autocert_path.new"
              mv "$autocert_path.new" "$autocert_path"
              grep -q 'return \[\]string{"http-01"}' "$autocert_path"
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
