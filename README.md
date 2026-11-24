<div align="center">

# 📻 LittleSDR

**Un récepteur SDR pour Android, pensé pour le tactile, avec son propre moteur radio en Rust.**
SDR, radio numérique DAB+ et DRM, images satellite Meteor : une seule app, une interface par usage.

![Android 9+](https://img.shields.io/badge/Android-9%2B-3DDC84?logo=android&logoColor=white)
![Kotlin](https://img.shields.io/badge/Kotlin-Compose-7F52FF?logo=kotlin&logoColor=white)
![Rust](https://img.shields.io/badge/Moteur-Rust-B7410E?logo=rust&logoColor=white)
![Vulkan](https://img.shields.io/badge/Rendu-Vulkan-AC162C?logo=vulkan&logoColor=white)
![Licence GPL-3.0](https://img.shields.io/badge/Licence-GPL--3.0--or--later-blue)

<img src="docs/screenshot-main.jpg" alt="LittleSDR en paysage : spectre et waterfall à gauche, panneau de contrôle à onglets à droite" width="860">

</div>

---

LittleSDR est une app Android de réception radio logicielle. Tout le traitement du signal, les décodeurs, la sortie son et l'affichage du spectre viennent d'un **moteur écrit en Rust** (`engine/`), sans C++ ; l'interface est en Kotlin / Jetpack Compose Material 3.

## ✨ Fonctionnalités

Un bouton change d'interface selon l'usage :

**SDR** : l'écran complet
- Démodulation NFM, WFM (stéréo, RDS), AM, DSB, USB, LSB, CW, IQ brut
- Spectre et waterfall en **Vulkan**, lissés, six palettes ; zoom, joystick d'accord
- Plan de bandes coloré selon la région UIT, préréglages (FM, LW, MW, HF, CB, VHF, UHF, PMR446), mémoires
- Horaires ondes courtes **EiBi**, RDS en FM
- **FT8 / FT4** (liste et carte des liaisons), **morse** (tous les signaux CW d'une bande), **SSTV** (ISS…)
- **Enregistrement** du son ou de l'IQ (bande entière, autour de la fréquence, ou largeur au choix)

**DAB+** : un autoradio
- Grand afficheur : logo ou diaporama de la station, nom, texte défilant, barres de signal
- Boutons ⏮ ▶ ⏭, stations en tuiles, **recherche par nom** dans tous les multiplex entendus
- Recherche automatique des multiplex (bande III, 5A à 13F)
- **Suivi de station (AF)** quand on se déplace, puis **repli FM** si la station n'est plus en DAB
- Logos des stations (annuaire radio-browser.info, Wi-Fi seulement par défaut)
- Portrait ou paysage (support de voiture)

**DRM** : radio numérique en ondes courtes (xHE-AAC), bandes de radiodiffusion, gain et bias-tee à portée de main

**LRPT** : images des satellites météo Meteor-M (137 MHz), affichées pendant le passage

## 📡 Récepteurs

| Liaison | Récepteurs |
|---|---|
| USB (OTG) | RTL-SDR (dont Blog V4, bias-tee) |
| Réseau | rtl_tcp, SpyServer, KiwiSDR (annuaire public, serveurs qui refusent les applications signalés) |
| Fichier | enregistrements IQ en WAV |

## 📥 Installation

Télécharge le dernier APK dans les [**Releases**](../../releases) et installe-le. Branche ton récepteur en OTG : l'app propose de s'ouvrir et sélectionne la source toute seule.

> Les versions jusqu'à la 1.2.0 étaient signées avec une clé de développement. Pour passer à une version signée avec la clé de publication, désinstalle une fois l'ancienne (les réglages sont perdus).

## 🛠️ Compiler

Prérequis :
- SDK Android avec le NDK `29.0.14206865`, un JDK 21
- Rust (`rustup`) avec les cibles Android et `cargo-ndk` :
  ```sh
  rustup target add aarch64-linux-android x86_64-linux-android
  cargo install cargo-ndk
  ```
- `cmake`, `ninja`, `git`, `curl`

```sh
git clone https://github.com/Radinux/LittleSDR.git
cd LittleSDR
kit/build-kit.sh                        # une fois : pilote RTL-SDR (librtlsdr, libusb)
engine/scripts/build-android-aac.sh     # une fois : décodeurs AAC (FAAD2 pour le DAB+, libxaac pour le DRM)
./gradlew assembleRelease               # app/build/outputs/apk/release/app-release.apk
```

Gradle compile le moteur Rust avec `cargo ndk`. Sans les décodeurs AAC, l'app se construit quand même mais le son DAB+ / DRM reste muet. Sans clé de publication (voir `app/build.gradle.kts`), l'APK est signé avec la clé de débogage.

Le moteur se compile et se teste aussi sur Linux, avec un outil en ligne de commande :

```sh
cd engine
cargo test --release
cargo run --release -- --help
```

## 🐧 Prototype Linux

Le même moteur sur le bureau, avec une interface egui : spectre et cascade (clic pour accorder), modes, réglages du récepteur, stations DAB+, RDS, DRM. Son par PipeWire / ALSA.

```sh
cd desktop
cargo run --release                                   # source et fréquence gardées d'une fois sur l'autre
cargo run --release -- --file capture.wav --mode 8    # un enregistrement IQ, en DAB+
```

Options : `--file <iq.wav>`, `--mode <0-10>` (NFM, WFM, AM, DSB, USB, CW, LSB, RAW, DAB+, DRM, LRPT), `--no-audio`. Les réglages vont dans `~/.local/share/littlesdr`. Il faut `libfaad` (paquet `faad2`) et, pour le RTL-SDR en USB, `librtlsdr`.

## 🧩 Architecture

```
Kotlin / Compose Material 3              app/src/main/java/app/littlesdr
 ├─ ui/        écrans (SDR, autoradio DAB+, DRM, LRPT), réglages, carte FT8
 ├─ engine/    façade vers le moteur (JNI), modèles
 ├─ data/      mémoires, préférences, EiBi, logos, position
 ├─ usb/       détection, permission, descripteur USB → pilote
 └─ service/   service au premier plan (écoute en arrière-plan)

Rust                                     engine/   (desktop/ : prototype Linux, egui)
 ├─ src/service/   couche pour l'app : sources, fil DSP, spectre, état, réglages
 ├─ src/android/   JNI, son AAudio, rendu Vulkan (ash), shaders GLSL
 ├─ src/source/    RTL-SDR, rtl_tcp, SpyServer, KiwiSDR, fichiers WAV
 ├─ src/dab/, drm/, lrpt/, ft8/, cw.rs, sstv.rs, rds.rs, recorder.rs …
 └─ scripts/build-android-aac.sh
```

## 🙏 Crédits

- [SDR++](https://github.com/AlexandreRouma/SDRPlusPlus) et [SDR++Brown](https://github.com/sannysanoff/SDRPlusPlusBrown) : premier moteur de l'app, base de données CTY des indicatifs
- [Dream](https://sourceforge.net/projects/drm/) et [gr-drm](https://github.com/kit-cel/gr-drm) : références pour le décodeur DRM
- [FAAD2](https://github.com/knik0/faad2) (AAC, HE-AAC) et [libxaac](https://github.com/ittiam-systems/libxaac) (xHE-AAC)
- [librtlsdr](https://github.com/osmocom/rtl-sdr) et [libusb](https://libusb.info/)
- [radio-browser.info](https://www.radio-browser.info/) : logos des stations
- [KiwiSDR](http://kiwisdr.com/public/) : annuaire public des récepteurs (via rx.linkfanel.net)
- [EiBi](http://www.eibispace.de/) : horaires des stations ondes courtes (usage non commercial)
- [Natural Earth](https://www.naturalearthdata.com/) : fond de carte (domaine public)
- Palettes Viridis, Inferno, Magma : approximations polynomiales de Matt Zucker ; Turbo : Google

## 📄 Licence

LittleSDR (app et moteur) est distribué sous **GPL-3.0-or-later**. Les bibliothèques liées gardent leur licence : FAAD2 (GPL-2.0-or-later), libxaac (Apache-2.0), librtlsdr (GPL-2.0), libusb (LGPL-2.1).
