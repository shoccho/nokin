# Third-Party Native Sources

Nokin downloads these native source dependencies with `./scripts/fetch-native.sh`. They are
compiled locally and intentionally excluded from Git.

| Dependency | Version | Official archive | SHA-256 |
| --- | --- | --- | --- |
| Scintilla | 5.6.2 | `https://www.scintilla.org/scintilla562.tgz` | `7b8345a224d7473b60c23face71ca8efb649c3b970705588911b40b505a0b10d` |
| Lexilla | 5.4.8 | `https://www.scintilla.org/lexilla548.tgz` | `742909e4f9c9d23ad2c4239185bf37977f35b0fb118daf52c1d0bcf7f8a79f29` |

Each extracted source tree contains its upstream `License.txt`. Nokin's MIT License does not
replace or modify those upstream licenses.
