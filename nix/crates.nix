# Auto-découverte des crates binaires de `crates/`.
# Un crate ayant au moins une cible [[bin]] devient un outil (package + app).
{ lib, root }:
let
  cratesDir = root + "/crates";
  dirs = lib.attrNames (lib.filterAttrs (_: type: type == "directory") (builtins.readDir cratesDir));

  read =
    dir:
    let
      manifest = cratesDir + "/${dir}/Cargo.toml";
    in
    lib.optionals (builtins.pathExists manifest) [
      (
        let
          toml = builtins.fromTOML (builtins.readFile manifest);
        in
        {
          inherit dir;
          name = toml.package.name;
          description = toml.package.description or toml.package.name;
          bins = map (b: b.name) (toml.bin or [ ]);
        }
      )
    ];

  # Binaire implicite (src/main.rs sans [[bin]]) pris en charge aussi.
  withImplicitBin =
    c:
    if c.bins == [ ] && builtins.pathExists (cratesDir + "/${c.dir}/src/main.rs") then
      c // { bins = [ c.name ]; }
    else
      c;
in
{
  inherit cratesDir;
  # [ { dir, name, description, bins } ]
  tools = lib.filter (c: c.bins != [ ]) (map withImplicitBin (lib.concatMap read dirs));
}
