# App icon

Electron-builder expects a macOS icon in `.icns` format.

## Where to put it

- Put your icon at: `build/icon.icns`
- It will be used by electron-builder because `package.json` sets `build.mac.icon`.

## How to generate `icon.icns`

Starting from a single square PNG (recommended 1024×1024) called `icon.png`:

```sh
mkdir -p build/icon.iconset
sips -z 16 16     icon.png --out build/icon.iconset/icon_16x16.png
sips -z 32 32     icon.png --out build/icon.iconset/icon_16x16@2x.png
sips -z 32 32     icon.png --out build/icon.iconset/icon_32x32.png
sips -z 64 64     icon.png --out build/icon.iconset/icon_32x32@2x.png
sips -z 128 128   icon.png --out build/icon.iconset/icon_128x128.png
sips -z 256 256   icon.png --out build/icon.iconset/icon_128x128@2x.png
sips -z 256 256   icon.png --out build/icon.iconset/icon_256x256.png
sips -z 512 512   icon.png --out build/icon.iconset/icon_256x256@2x.png
sips -z 512 512   icon.png --out build/icon.iconset/icon_512x512.png
sips -z 1024 1024 icon.png --out build/icon.iconset/icon_512x512@2x.png
iconutil -c icns build/icon.iconset -o build/icon.icns
rm -rf build/icon.iconset
```

Then rebuild:

```sh
npm run build:mac
```
