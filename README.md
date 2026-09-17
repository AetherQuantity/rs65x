# rs65x

Obligatory 6502 emulator learning project

## Features

- Cycle accuracy!
- Passes NMOS and CMOS Klaus2m5 functional tests!
- Passes all applicable SingleStepTests (65x02)!
- NMOS, NES, Synertek 65C02, Rockwell 65C02, and WDC 65C02 instruction sets
- Not suitable for actually writing an emulator with because it's still slow and buggy!!!

## TODOs
- CMOS WAI/STP behavior
- I'm not a trillion percent confident on the accuracy of external signals like IRQ.

## Running the tests

Download the [SingleStepTests 65x02 repository ZIP](https://github.com/SingleStepTests/65x02/archive/refs/heads/main.zip)
and place it in `tests/data/` as `65x02.zip` or `65x02-main.zip`, then run:

```sh
cargo test
```

The loader checks `65x02.zip` first, then `65x02-main.zip`. It reads one opcode
at a time directly from the archive without extracting files to disk. Either
archive name may contain a `65x02/` or `65x02-main/` root directory.

If neither archive exists, you can instead extract the repository into
`tests/data/65x02/` or `tests/data/65x02-main/` (checked in that order per CPU
suite). Preserve the upstream layout, including the `v1` directories:

```text
tests/data/65x02-main/
  6502/v1/00.json ... ff.json
  nes6502/v1/00.json ... ff.json
  synertek65c02/v1/00.json ... ff.json
  rockwell65c02/v1/00.json ... ff.json
  wdc65c02/v1/00.json ... ff.json
```

Missing files, unreadable or corrupt archives, and invalid or empty test JSON
fail the tests. Once a source is selected for a suite, loading errors never
fall back to another source. Test data is kept outside version control; use the
same upstream revision when comparing results between machines.

The `wdc_tests`, `rockwell_tests`, `synertek_tests`, `nes_tests`, and
`nmos_tests` modules correspond to the five upstream suites. Synertek and
Rockwell cover all 256 opcodes. The WDC sweep
explicitly excludes WAI (`CB`) and STP (`DB`), whose upstream files are empty;
their execution behavior still needs implementation and dedicated tests.
