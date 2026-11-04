// A stub of the Words facade (crates/desktop/src/words.rs, over pg_client's text): the rating symbols and the timecode,
// with the same names and the same answers as the Rust (text::RATING_SYMBOLS, text::timecode).
import QtQuick

QtObject {
    readonly property var symbols: ["☆", "❌", "❔", "🔨", "✔️", "⭐"]

    function ratingSymbol(rating) { return symbols[Math.min(Math.max(rating, 0), 5)] }

    function timecode(seconds) {
        const h = Math.floor(seconds / 3600)
        const m = Math.floor(seconds / 60) % 60
        const s = seconds % 60
        const pad = (n) => (n < 10 ? "0" : "") + n
        return h > 0 ? h + ":" + pad(m) + ":" + pad(s) : m + ":" + pad(s)
    }
}
