// Inline line-icon set for the desktop chrome. One component, one name map, so
// every region renders icons the same way (24px grid, 1.6px currentColor
// strokes) and a new region only needs a name.
import React from 'react';

const P = {
  app: <><path d="M12 3 20 7.5v9L12 21 4 16.5v-9z" /><path d="M12 7.6l4 2.3v4.6l-4 2.3-4-2.3V9.9z" /></>,
  new: <><path d="M6 3h7l5 5v13H6z" /><path d="M13 3v5h5M12 12v6M9 15h6" /></>,
  open: <><path d="M3 7a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z" /><path d="M3 11h18" /></>,
  save: <><path d="M5 3h11l3 3v15H5z" /><path d="M8 3v6h7V3" /><path d="M8 21v-7h8v7" /></>,
  undo: <><path d="M4 12h11a5 5 0 0 1 0 10h-4" /><path d="M9 7l-5 5 5 5" /></>,
  redo: <><path d="M20 12H9a5 5 0 0 0 0 10h4" /><path d="M15 7l5 5-5 5" /></>,
  fit: <><path d="M4 9V5h4M20 9V5h-4M4 15v4h4M20 15v4h-4" /><rect x="8.5" y="8.5" width="7" height="7" /></>,
  zoomIn: <><circle cx="11" cy="11" r="6" /><path d="M20 20l-4.4-4.4M11 8.5v5M8.5 11h5" /></>,
  zoomOut: <><circle cx="11" cy="11" r="6" /><path d="M20 20l-4.4-4.4M8.5 11h5" /></>,
  zoomWindow: <><rect x="4" y="4" width="10" height="8" /><circle cx="15" cy="15" r="4.5" /><path d="M20 20l-3-3" /></>,
  pan: <><path d="M12 3v18M3 12h18" /><path d="M9 6l3-3 3 3M9 18l3 3 3-3M6 9l-3 3 3 3M18 9l3 3-3 3" /></>,
  orbit: <><circle cx="12" cy="12" r="8" /><ellipse cx="12" cy="12" rx="8" ry="3.4" /><path d="M12 4v16" /></>,
  view: <><path d="M12 3 20 7.5v9L12 21 4 16.5v-9z" /><path d="M12 12 20 7.5M12 12v9M12 12 4 7.5" /></>,
  grid: <><rect x="4" y="4" width="16" height="16" /><path d="M4 9.3h16M4 14.6h16M9.3 4v16M14.6 4v16" /></>,
  axes: <><path d="M6 18V6M6 18h12" /><path d="M6 6l-2.2 2.2M6 6l2.2 2.2M18 18l-2.2-2.2M18 18l-2.2 2.2" /></>,
  snap: <><path d="M12 3v6M12 15v6M3 12h6M15 12h6" /><rect x="9" y="9" width="6" height="6" /></>,
  line: <><circle cx="5" cy="19" r="2" /><circle cx="19" cy="5" r="2" /><path d="M6.6 17.4 17.4 6.6" /></>,
  circle: <><circle cx="12" cy="12" r="8" /><circle cx="12" cy="12" r="1" /></>,
  arc: <><path d="M4 18A14 14 0 0 1 20 18" /><circle cx="4" cy="18" r="1.4" /><circle cx="20" cy="18" r="1.4" /></>,
  move: <><path d="M12 3v18M3 12h18" /><path d="M9 6l3-3 3 3M9 18l3 3 3-3M6 9l-3 3 3 3M18 9l3 3-3 3" /></>,
  copy: <><rect x="8" y="8" width="12" height="12" rx="1" /><path d="M4 16V4h12" /></>,
  trim: <><circle cx="6" cy="7" r="2.2" /><circle cx="6" cy="17" r="2.2" /><path d="M8 8.4 20 18M8 15.6 20 6" /></>,
  extend: <><path d="M4 12h9" /><path d="M10.5 9 13.5 12l-3 3" /><path d="M20 5v14" /></>,
  rotate: <><path d="M20 12a8 8 0 1 1-2.4-5.7" /><path d="M20 4v4h-4" /></>,
  scale: <><path d="M4 20V9h11" /><path d="M15 4h5v5" /><path d="M4.5 19.5 20 4" /></>,
  mirror: <><path d="M12 3v18" strokeDasharray="3 3" /><path d="M9 6 4 12l5 6z" /><path d="M15 6l5 6-5 6z" /></>,
  offset: <><path d="M4 17a13 13 0 0 1 16 0" /><path d="M8 17a9 9 0 0 1 8 0" /></>,
  chamfer: <><path d="M4 20V10" /><path d="M10 4h10" /><path d="M4 10 10 4" /></>,
  fillet: <><path d="M4 20V10" /><path d="M10 4h10" /><path d="M4 10a6 6 0 0 1 6-6" /></>,
  arrayrect: <><rect x="3.5" y="3.5" width="6" height="6" /><rect x="14.5" y="3.5" width="6" height="6" /><rect x="3.5" y="14.5" width="6" height="6" /><rect x="14.5" y="14.5" width="6" height="6" /></>,
  arraypolar: <><circle cx="12" cy="12" r="2" /><circle cx="12" cy="3.5" r="1.6" /><circle cx="19.4" cy="16" r="1.6" /><circle cx="4.6" cy="16" r="1.6" /><path d="M12 5.1V10M18 15.2l-4.4-1.7M6 15.2l4.4-1.7" /></>,
  align: <><circle cx="4.5" cy="18" r="1.7" /><circle cx="10.5" cy="18" r="1.7" /><circle cx="13.5" cy="6" r="1.7" /><circle cx="19.5" cy="6" r="1.7" /><path d="M4.5 18 13.5 6M10.5 18 19.5 6" /></>,
  stretch: <><rect x="3.5" y="8" width="6" height="8" /><rect x="14.5" y="8" width="6" height="8" strokeDasharray="2 2" /><path d="M10 12h4M12.5 10l2 2-2 2" /></>,
  erase: <><path d="M9 20.5H6.9l-3.3-3.3a1.8 1.8 0 0 1 0-2.5l8.9-8.9a1.8 1.8 0 0 1 2.5 0l3.3 3.3a1.8 1.8 0 0 1 0 2.5L11.5 20.5z" /><path d="M7.2 13.8 12.2 18.8" /><path d="M4 20.5h16" /></>,
  explode: <><circle cx="12" cy="12" r="2" /><path d="M12 6.5V3M12 21v-3.5M6.5 12H3M21 12h-3.5M8.1 8.1 5.6 5.6M18.4 18.4l-2.5-2.5M15.9 8.1l2.5-2.5M8.1 15.9l-2.5 2.5" /></>,
  distance: <><path d="M4 8v8M20 8v8M4 12h16" /><path d="M7 9 4 12l3 3M17 9l3 3-3 3" /></>,
  polyline: <><path d="M4 18 9 8l5 6 6-10" /><circle cx="4" cy="18" r="1.5" /><circle cx="9" cy="8" r="1.5" /><circle cx="20" cy="4" r="1.5" /></>,
  rectang: <><rect x="4" y="6" width="16" height="12" /><circle cx="4" cy="6" r="1.5" /><circle cx="20" cy="18" r="1.5" /></>,
  ellipse: <><ellipse cx="12" cy="12" rx="9" ry="5.2" /><circle cx="12" cy="12" r="1.2" /><circle cx="3" cy="12" r="1.4" /><circle cx="21" cy="12" r="1.4" /></>,
  polygon: <><path d="M12 3.5 19.5 9l-2.9 8.8H7.4L4.5 9z" /><circle cx="12" cy="12" r="1.2" /><circle cx="12" cy="3.5" r="1.4" /></>,
  point: <><circle cx="12" cy="12" r="2.2" /><path d="M12 4v4M12 16v4M4 12h4M16 12h4" /></>,
  spline: <><path d="M3 16c3-7 6 7 9 0s6-7 9 0" /><circle cx="3" cy="16" r="1.4" /><circle cx="12" cy="16" r="1.4" /><circle cx="21" cy="16" r="1.4" /></>,
  angle: <><path d="M4 19h16M4 19 16 5" /><path d="M12 19a8 8 0 0 0-2-5.3" /></>,
  area: <><path d="M5 6h14l-2 12H7z" strokeDasharray="3 2" /></>,
  layers: <><path d="M12 4 3 9l9 5 9-5z" /><path d="M4 13.5 12 18l8-4.5" /></>,
  properties: <><path d="M4 7h8M4 12h13M4 17h6" /><circle cx="17" cy="7" r="1.9" /><circle cx="15" cy="17" r="1.9" /></>,
  selectAll: <><path d="M4 4h6M14 4h6M4 20h6M14 20h6M4 4v6M4 14v6M20 4v6M20 14v6" /><path d="M9 12.5l2.4 2.4L16 10" /></>,
  clearSelection: <><circle cx="12" cy="12" r="8" strokeDasharray="3 2" /><path d="M9 9l6 6M15 9l-6 6" /></>,
  diagnostics: <><path d="M3 12h3.5l2-5.5L12 18l2-7 1.5 1H21" /></>,
  settings: <><circle cx="12" cy="12" r="3" /><path d="M12 3v2M12 19v2M3 12h2M19 12h2M5.6 5.6l1.4 1.4M17 17l1.4 1.4M18.4 5.6 17 7M7 17l-1.4 1.4" /></>,
  search: <><circle cx="11" cy="11" r="6" /><path d="M20 20l-4.4-4.4" /></>,
  close: <><path d="M6 6l12 12M18 6 6 18" /></>,
  plus: <><path d="M12 5v14M5 12h14" /></>,
  minus: <><path d="M5 12h14" /></>,
  chevronDown: <><path d="M6 9.5l6 6 6-6" /></>,
  chevronUp: <><path d="M6 14.5l6-6 6 6" /></>,
  chevronRight: <><path d="M9.5 5l7 7-7 7" /></>,
  chevronLeft: <><path d="M14.5 5l-7 7 7 7" /></>,
  launcher: <><path d="M6 6h13v13H6z" opacity=".35" /><path d="M14 14h5v5" /><path d="M19 14 9 4" /></>,
  cleanScreen: <><path d="M9 4H4v5M15 4h5v5M9 20H4v-5M15 20h5v-5" /></>,
  terminal: <><rect x="3.5" y="5" width="17" height="14" rx="1" /><path d="M7 10l3 2-3 2M12.5 14h4" /></>,
  palette: <><rect x="4" y="5" width="16" height="14" rx="1" /><path d="M10 5v14" /></>,
  language: <><circle cx="12" cy="12" r="8" /><path d="M4 12h16M12 4a12 12 0 0 1 0 16 12 12 0 0 1 0-16z" /></>,
  more: <><circle cx="6" cy="12" r="1.5" /><circle cx="12" cy="12" r="1.5" /><circle cx="18" cy="12" r="1.5" /></>,
  start: <><path d="M4 11 12 4l8 7" /><path d="M6 10v10h12V10" /></>,
  drawing: <><rect x="4" y="5" width="16" height="14" rx="1" /><path d="M4 16l4-4 3 3 3-4 6 6" /></>,
  alert: <><path d="M12 4 21 20H3z" /><path d="M12 10v5M12 17.6h.01" /></>,
  spinner: <><path d="M12 4a8 8 0 1 1-8 8" /></>,
  check: <><path d="M5 13l4.5 4.5L19 7" /></>,
  dot: <><circle cx="12" cy="12" r="2.5" /></>,
};

export function Icon({ name, size = 20, className, title, ...rest }) {
  const paths = P[name] ?? P.dot;
  return (
    <svg
      className={['icon', className].filter(Boolean).join(' ')}
      width={size}
      height={size}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.6"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden={title ? undefined : 'true'}
      role={title ? 'img' : undefined}
      focusable="false"
      {...rest}
    >
      {title ? <title>{title}</title> : null}
      {paths}
    </svg>
  );
}
