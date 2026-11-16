self: homeManager:
{ config, lib, pkgs, ... }:
let
  cfg = config.services.bitnet;
  service = {
    description = "BitNet desktop client";
    command = "${cfg.package}/bin/bitnet-gui" +
      lib.optionalString cfg.silentStart " --silent";
    session = "graphical-session";
  };
in {
  options.services.bitnet = {
    enable = lib.mkEnableOption "BitNet desktop client";
    package = lib.mkPackageOption self.packages.${pkgs.stdenv.hostPlatform.system} "bitnet" { };
    autoStart = lib.mkOption {
      type = lib.types.bool;
      default = false;
      description = "Start BitNet when the graphical user session starts.";
    };
    silentStart = lib.mkOption {
      type = lib.types.bool;
      default = false;
      description = "Start BitNet in the tray when the desktop offers a tray.";
    };
  };
  config = lib.mkIf cfg.enable (
    if homeManager then {
      home.packages = [ cfg.package ];
      systemd.user.services.bitnet = lib.mkIf cfg.autoStart {
        Unit = {
          Description = service.description;
          PartOf = [ "${service.session}.target" ];
        };
        Service.ExecStart = service.command;
        Install.WantedBy = [ "${service.session}.target" ];
      };
    } else {
      environment.systemPackages = [ cfg.package ];
      systemd.user.services.bitnet = lib.mkIf cfg.autoStart {
        description = service.description;
        partOf = [ "${service.session}.target" ];
        wantedBy = [ "${service.session}.target" ];
        serviceConfig.ExecStart = service.command;
      };
    }
  );
}
