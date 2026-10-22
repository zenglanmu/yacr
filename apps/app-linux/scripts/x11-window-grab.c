/* Grab an X11 window's own drawable (XGetImage on the window, NOT the root)
 * and write a PPM. Method note: this reads the window's backing pixmap via the
 * X protocol, so it is a different path from an X11 *root* grab (which is black
 * under rootless XWayland). A black result here means the window itself is not
 * readable, which is a distinct finding.
 *
 * Usage note (Wine diagnostic only): the same tool can capture the **top-level
 * X11 window that Wine creates** for a Windows host (`x11-window-grab <xid-hex>
 * <out.ppm>`), which is useful to prove the process created a mapped window.
 * It captures Wine's X11 drawable, **not** the Windows compositor/WebView2/DComp
 * result, so its pixels are NOT a visual verification of the Windows host and
 * must be labelled `NOT VERIFIED (Wine limitation)`.
 *
 * Build: gcc -O2 -o x11-window-grab x11-window-grab.c -lX11
 * Usage: x11-window-grab <xid-hex> <out.ppm>
 */
#include <X11/Xlib.h>
#include <X11/Xutil.h>
#include <stdio.h>
#include <stdlib.h>

int main(int argc, char **argv) {
    if (argc < 3) {
        fprintf(stderr, "usage: %s <xid-hex> <out.ppm>\n", argv[0]);
        return 2;
    }
    Window w = (Window)strtoul(argv[1], NULL, 0);
    Display *d = XOpenDisplay(NULL);
    if (!d) {
        fprintf(stderr, "XOpenDisplay failed\n");
        return 3;
    }
    XWindowAttributes a;
    if (!XGetWindowAttributes(d, w, &a)) {
        fprintf(stderr, "XGetWindowAttributes failed for 0x%lx\n", (unsigned long)w);
        XCloseDisplay(d);
        return 4;
    }
    XImage *img = XGetImage(d, w, 0, 0, a.width, a.height, AllPlanes, ZPixmap);
    if (!img) {
        fprintf(stderr, "XGetImage failed (window not readable) 0x%lx\n", (unsigned long)w);
        XCloseDisplay(d);
        return 5;
    }
    FILE *f = fopen(argv[2], "wb");
    if (!f) {
        fprintf(stderr, "cannot open output %s\n", argv[2]);
        XDestroyImage(img);
        XCloseDisplay(d);
        return 6;
    }
    fprintf(f, "P6\n%d %d\n255\n", img->width, img->height);
    int rshift = __builtin_ctzl(img->red_mask);
    int gshift = __builtin_ctzl(img->green_mask);
    int bshift = __builtin_ctzl(img->blue_mask);
    for (int y = 0; y < img->height; y++) {
        for (int x = 0; x < img->width; x++) {
            unsigned long p = XGetPixel(img, x, y);
            unsigned char r = (unsigned char)((p & img->red_mask) >> rshift);
            unsigned char g = (unsigned char)((p & img->green_mask) >> gshift);
            unsigned char b = (unsigned char)((p & img->blue_mask) >> bshift);
            fputc(r, f);
            fputc(g, f);
            fputc(b, f);
        }
    }
    fclose(f);
    XDestroyImage(img);
    XCloseDisplay(d);
    fprintf(stderr, "wrote %s (%dx%d)\n", argv[2], a.width, a.height);
    return 0;
}
