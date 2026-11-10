// The saved views menu (ImageRater's bookmarks): the views by name, to apply one; the save of the current view; and
// the delete. It reports what the user asks for (signals); TopBar passes them on, and Main makes the calls.
pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Controls.Fusion
import pg.gallery

Button {
    Words {
        id: words
    }
    id: views

    // The saved views, by name, and whether there are any (the delete menu is off without).
    required property var savedViews
    required property bool hasSavedViews

    // The menu is open (a modal panel: Main reports it, PanelKind.ViewsMenu).
    readonly property bool menuOpen: viewsMenu.visible

    signal viewChosen(string name)
    signal viewDeleteRequested(string name)
    signal saveViewRequested()

    // Opens the menu (the test hook --menu=views).
    function open() {
        views.clicked()
    }

    text: words.text("ui-views")
    onClicked: viewsMenu.popup(views, 0, views.height)

    Menu {
        id: viewsMenu
        Instantiator {
            model: views.savedViews
            delegate: MenuItem {
                required property string modelData
                text: modelData
                onTriggered: views.viewChosen(modelData)
            }
            onObjectAdded: (i, o) => viewsMenu.insertItem(i, o as MenuItem)
            onObjectRemoved: (i, o) => viewsMenu.removeItem(o as MenuItem)
        }
        MenuSeparator {}
        MenuItem {
            text: words.text("ui-save-current-view")
            onTriggered: views.saveViewRequested()
        }
        Menu {
            id: deleteMenu
            title: words.text("ui-delete-view")
            enabled: views.hasSavedViews
            Instantiator {
                model: views.savedViews
                delegate: MenuItem {
                    required property string modelData
                    text: modelData
                    onTriggered: views.viewDeleteRequested(modelData)
                }
                onObjectAdded: (i, o) => deleteMenu.insertItem(i, o as MenuItem)
                onObjectRemoved: (i, o) => deleteMenu.removeItem(o as MenuItem)
            }
        }
    }
}
