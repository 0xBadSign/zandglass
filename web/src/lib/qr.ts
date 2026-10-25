import qrcode from 'qrcode-generator';
import { svg } from './dom';

/** A QR code as an SVG (black on transparent; place it on a light tile). */
export function qrSvg(text: string, label: string, ecc: 'L' | 'M' = 'M'): SVGElement {
  const qr = qrcode(0, ecc);
  qr.addData(text);
  qr.make();
  const n = qr.getModuleCount();
  let d = '';
  for (let r = 0; r < n; r++) {
    for (let c = 0; c < n; c++) {
      if (qr.isDark(r, c)) d += `M${c + 1} ${r + 1}h1v1h-1z`;
    }
  }
  const s = svg('svg', {
    viewBox: `0 0 ${n + 2} ${n + 2}`,
    role: 'img',
    'aria-label': label,
    'shape-rendering': 'crispEdges',
  });
  s.append(svg('path', { d, fill: '#000' }));
  return s;
}
