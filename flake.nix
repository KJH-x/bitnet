{
  description = "BitNet desktop client and personal engine for the BIT campus network";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-26.05";

  outputs = { self, nixpkgs }:
    let
      systems = [ "x86_64-linux" "aarch64-linux" ];
      forSystems = nixpkgs.lib.genAttrs systems;
      module = import ./nix/user-module.nix self;
    in {
      packages = forSystems (system: {
        default = nixpkgs.legacyPackages.${system}.callPackage ./nix/package.nix { };
        bitnet = self.packages.${system}.default;
      });
      apps = forSystems (system: {
        default = {
          type = "app";
          program = "${self.packages.${system}.default}/bin/bitnet-gui";
        };
        tui = {
          type = "app";
          program = "${self.packages.${system}.default}/bin/bitnet-tui";
        };
        web = {
          type = "app";
          program = "${self.packages.${system}.default}/bin/bitnet-web";
        };
      });
      nixosModules.default = module false;
      homeManagerModules.default = module true;
    };
}
