// The rating symbols (ImageRater's), used by the grid badges, the viewer's rating bar and its popup, and the filter.
pragma Singleton
import QtQml
import QtQuick
import pg.gallery

Item {
    // The symbols are pg-client's text, through the Words facade: the filter line names the ratings with the same ones.
    Words {
        id: words
    }

    function symbol(rating: int): string {
        return words.ratingSymbol(rating)
    }
}
