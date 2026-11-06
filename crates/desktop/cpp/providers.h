// QQuickAsyncImageProvider front end for the Rust image loaders (src/providers.rs).
#pragma once

#include <QtQml/QQmlImageProviderBase>

#include <cstdint>

namespace pg {

// providers.rs's ImageKind, declared as cxx generates it (repr u8). This header is included before cxx's own
// declarations, so the type is only forward-declared here.
enum class ImageKind : std::uint8_t;

// A new provider of the given kind; the QML engine takes ownership.
QQmlImageProviderBase* new_image_provider(ImageKind kind);

}  // namespace pg
