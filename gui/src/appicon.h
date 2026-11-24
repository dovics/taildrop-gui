#pragma once

#include <QIcon>

// The app's icon, from the artwork compiled into the binary.
//
// Why not QIcon::fromTheme(): that is a Linux desktop idea. Windows has no icon
// theme and no lookup by name, so a name-only icon is an empty square that still
// claims to be visible — a tray icon nobody can see and nobody can click.
//
// The PNGs are the same drawing as gui/pkgbuild/taildrop-gui.svg, at the sizes
// Windows asks for (16 in the taskbar's small mode up to 256 in Explorer), and
// adding every size lets Qt pick the sharp one instead of scaling a big bitmap
// down.
inline QIcon taildropIcon() {
    static const int sizes[] = {16, 22, 32, 48, 64, 128, 256};
    QIcon icon;
    for (int size : sizes)
        icon.addFile(QStringLiteral(":/icons/taildrop-%1.png").arg(size),
                     QSize(size, size));
    if (icon.isNull())
        icon = QIcon::fromTheme("taildrop-gui");
    return icon;
}
