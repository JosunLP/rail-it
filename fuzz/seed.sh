#!/bin/sh
# Startkorpus für cargo-fuzz aus den Beispielen (aus dem Wurzelverzeichnis aufrufen).
set -eu
railit=${RAILIT:-target/release/railit}
mkdir -p fuzz/corpus/planpro_network
cp examples/abzweig.ppxml fuzz/corpus/planpro_network/
"$railit" net export examples/beispielstadt.toml --out fuzz/corpus/planpro_network/beispielstadt.ppxml
mkdir -p fuzz/corpus/plan_image fuzz/corpus/toml_network fuzz/corpus/railml_network \
         fuzz/corpus/timetable fuzz/corpus/journal
cp examples/beispielstadt.toml examples/nordbahn.toml fuzz/corpus/toml_network/
cp crates/railit-import/tests/data/abzweig.railml fuzz/corpus/railml_network/
cp examples/nordbahn-fahrplan.toml fuzz/corpus/timetable/
"$railit" plan compile examples/beispielstadt.toml --out fuzz/corpus/plan_image/beispielstadt.rplan >/dev/null
"$railit" plan compile examples/nordbahn.toml --out fuzz/corpus/plan_image/nordbahn >/dev/null
mv fuzz/corpus/plan_image/nordbahn/*.rplan fuzz/corpus/plan_image/ && rmdir fuzz/corpus/plan_image/nordbahn
"$railit" net export examples/beispielstadt.toml --out fuzz/corpus/railml_network/beispielstadt.railml
