{
  lib,
  writeShellApplication,
}:

writeShellApplication {
  name = "hello";

  text = builtins.readFile ./script.sh;

  meta = with lib; {
    description = "Affiche Hello World !";
    license = licenses.mit;
  };
}
