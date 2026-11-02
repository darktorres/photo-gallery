// QQuickAsyncImageProvider front end for the Rust image loaders (src/providers.rs).
#pragma once

#include <QtQml/QQmlImageProviderBase>

namespace pg {

// A new provider of the given kind (providers.rs KIND_*); the QML engine takes ownership.
QQmlImageProviderBase* new_image_provider(int kind);

}  // namespace pg
