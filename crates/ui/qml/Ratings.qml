// The rating symbols (ImageRater's), used by the grid badges, the viewer's rating bar and its popup, and the filter.
pragma Singleton
import QtQml

QtObject {
    readonly property list<string> symbols: ["☆", "❌", "❔", "🔨", "✔️", "⭐"]

    function symbol(rating: int): string {
        return symbols[Math.max(0, Math.min(5, rating))]
    }
}
