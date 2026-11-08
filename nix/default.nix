# Packaging des outils Rust : { packages, checks } pour un `pkgs` donné.
{
  pkgs,
  root ? ../.,
}:
let
  inherit (pkgs) lib;
  discovery = import ./crates.nix { inherit lib root; };

  src = lib.fileset.toSource {
    inherit root;
    fileset = lib.fileset.unions [
      (root + "/Cargo.toml")
      (root + "/Cargo.lock")
      (root + "/crates")
    ];
  };

  cargoLock.lockFile = root + "/Cargo.lock";

  version = (builtins.fromTOML (builtins.readFile (root + "/Cargo.toml"))).workspace.package.version;

  # Bibliothèques chargées dynamiquement (dlopen) par winit/wgpu/iced.
  guiLibs = with pkgs; [
    wayland
    libxkbcommon
    libGL
    vulkan-loader
    libx11
    libxcursor
    libxi
    libxrandr
    libxcb
    fontconfig
    freetype
  ];

  # Binaires externes appelés par `nixos-flakes-core` via PATH.
  # (suffixe : ne masque pas les wrappers setuid / le système.)
  runtimePath = lib.makeBinPath [
    pkgs.git
    pkgs.nix
    pkgs.libnotify
  ];
  systemPath = "/run/wrappers/bin:/run/current-system/sw/bin";

  # Réglages propres à certains outils, indexés par nom de crate.
  extras = {
    nixos-flakes = { };
    nixos-flakes-gui = {
      buildInputs = guiLibs;
      nativeBuildInputs = [
        pkgs.patchelf
        pkgs.copyDesktopItems
      ];
      desktopItems = [
        (pkgs.makeDesktopItem {
          name = "nixos-flakes-gui";
          desktopName = "Gestion des flakes";
          genericName = "Gestionnaire de flakes NixOS";
          comment = "Mettre à jour et appliquer les flakes de la machine";
          exec = "nixos-flakes-gui";
          terminal = false;
          categories = [
            "System"
            "Settings"
          ];
          icon = "nixos-flakes-gui";
          startupWMClass = "nixos-flakes-gui";
        })
      ];
      postInstall = ''
        install -Dm644 crates/nixos-flakes-gui/assets/icons/nixos-flakes-gui.svg \
          $out/share/icons/hicolor/scalable/apps/nixos-flakes-gui.svg
      '';
      postFixup = ''
        for bin in $out/bin/*; do
          patchelf --add-rpath ${lib.makeLibraryPath guiLibs} "$bin"
        done
      '';
    };
  };

  mkTool =
    crate:
    let
      extra = extras.${crate.name} or { };
    in
    pkgs.rustPlatform.buildRustPackage (
      {
        pname = crate.name;
        inherit version src cargoLock;
        cargoBuildFlags = [
          "-p"
          crate.name
        ];
        cargoTestFlags = [
          "-p"
          crate.name
        ];
        nativeBuildInputs = [ pkgs.makeWrapper ] ++ (extra.nativeBuildInputs or [ ]);
        buildInputs = extra.buildInputs or [ ];
        nativeCheckInputs = [ pkgs.git ];
        preCheck = ''
          export HOME=$TMPDIR
          git config --global user.email test@example.org
          git config --global user.name test
          git config --global init.defaultBranch main
        '';
        postInstall = extra.postInstall or "";
        postFixup = (extra.postFixup or "") + ''
          for bin in $out/bin/*; do
            wrapProgram "$bin" \
              --suffix PATH : ${runtimePath}:${systemPath}
          done
        '';
        meta = {
          inherit (crate) description;
          license = lib.licenses.gpl3Plus;
          mainProgram = lib.head crate.bins;
          platforms = lib.platforms.linux;
        };
      }
      // lib.optionalAttrs (extra ? desktopItems) { inherit (extra) desktopItems; }
    );

  tools = lib.listToAttrs (map (c: lib.nameValuePair c.name (mkTool c)) discovery.tools);

  default = pkgs.symlinkJoin {
    name = "nixos-flakes-all-${version}";
    paths = lib.attrValues tools;
    meta.description = "Tous les outils de nixos-flakes";
  };

  # Dérivation de contrôle sur tout le workspace (sources + dépendances vendorées).
  workspaceCheck =
    name: extraInputs: script:
    pkgs.rustPlatform.buildRustPackage {
      pname = "nixos-flakes-${name}";
      inherit version src cargoLock;
      nativeBuildInputs = extraInputs;
      buildInputs = guiLibs;
      doCheck = false;
      dontCargoCheck = true;
      buildPhase = ''
        runHook preBuild
        export HOME=$TMPDIR
        ${script}
        runHook postBuild
      '';
      installPhase = "mkdir -p $out";
      dontFixup = true;
    };

  checks = {
    build = default;
    tests = pkgs.rustPlatform.buildRustPackage {
      pname = "nixos-flakes-tests";
      inherit version src cargoLock;
      buildInputs = guiLibs;
      nativeCheckInputs = [ pkgs.git ];
      cargoBuildFlags = [ "--workspace" ];
      cargoTestFlags = [ "--workspace" ];
      doCheck = true;
      preCheck = ''
        export HOME=$TMPDIR
        git config --global user.email test@example.org
        git config --global user.name test
        git config --global init.defaultBranch main
      '';
      installPhase = "mkdir -p $out";
      dontFixup = true;
    };
    clippy = workspaceCheck "clippy" [ pkgs.clippy ] ''
      cargo clippy --workspace --all-targets --offline -- -D warnings
    '';
    rustfmt =
      pkgs.runCommand "nixos-flakes-rustfmt"
        {
          nativeBuildInputs = [
            pkgs.rustfmt
            pkgs.findutils
          ];
        }
        ''
          find ${src}/crates -name '*.rs' -print0 | xargs -0 rustfmt --check --edition 2021
          mkdir -p $out
        '';
  };
in
{
  packages = tools // {
    inherit default;
  };
  inherit checks;
  # Pour l'overlay : seulement les outils, sans `default`.
  inherit tools;
  inherit guiLibs;
}
