# LittleSDR

Récepteur SDR pour Android, en Kotlin avec une interface Jetpack Compose Material 3 et un waterfall rendu en Vulkan.

LittleSDR est une refonte de l'interface, pas un fork : le traitement du signal et les pilotes viennent tels quels du moteur de [SDR++Brown](https://github.com/sannysanoff/SDRPlusPlusBrown) (lui-même basé sur SDR++), intégré comme sous-module git sans modification.

## Architecture

```
Kotlin / Compose Material 3        app/src/main/java/app/littlesdr
  ├─ ui/            écrans, afficheur de fréquence, volets, gestes
  ├─ engine/        Engine (thread unique), NativeBridge (JNI), modèle SmGui
  ├─ usb/           détection, permission, descripteur USB → pilotes
  ├─ data/          fréquences mémorisées, préférences (DataStore JSON)
  └─ service/       service au premier plan (écoute écran éteint)

C++ (NDK)                          app/src/main/cpp
  ├─ engine.cpp          démarrage du moteur sans interface (équivalent de MainWindow::init sans le dessin)
  ├─ headless_backend    API backend:: de SDR++ (fd USB, httpGet…) sans NativeActivity
  ├─ smgui_json.cpp      menus des récepteurs (SmGui) → JSON → widgets Compose
  ├─ spectrum_renderer   Vulkan : spectre + waterfall (palette Turbo calculée dans le shader)
  ├─ modules/aaudio_sink sortie audio AAudio
  └─ jni_bridge.cpp

third_party/sdrpp-brown            moteur SDR++Brown (sous-module, non modifié)
kit/build-kit.sh                   dépendances natives (libusb, volk, fftw, zstd, pilotes)
```

Récepteurs pris en charge : RTL-SDR, Airspy, Airspy HF+, HackRF (USB OTG), rtl_tcp, SpyServer, KiwiSDR (réseau).

## Compiler

Prérequis : SDK Android avec le NDK `29.0.14206865` et CMake `4.1.2` (via `sdkmanager`), un JDK 21 (Gradle le cherche dans `~/.jdks`).

```sh
git submodule update --init --recursive
kit/build-kit.sh                 # une seule fois : compile les dépendances natives (arm64-v8a, x86_64)
./gradlew assembleDebug          # APK : app/build/outputs/apk/debug/app-debug.apk
```

Le projet s'ouvre aussi directement dans Android Studio. Le JDK du daemon Gradle est fixé à 21 dans `gradle/gradle-daemon-jvm.properties`.

## Tester

- **Émulateur** : utiliser une source réseau, par exemple RTL-TCP vers un `rtl_tcp` du réseau local, ou SpyServer.
- **Téléphone** : brancher le récepteur en OTG. L'app propose de s'ouvrir, sélectionne la bonne source et rescanne les périphériques.

## Licence

Le moteur SDR++Brown est sous GPL-3.0. LittleSDR, qui s'y lie, est distribué sous la même licence.
