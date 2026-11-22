/**
 * An OKLCH colour as a six-digit sRGB hex colour, each channel clipped to the
 * sRGB range. Lightness is 0 to 1, chroma as CSS takes it, hue in degrees.
 * The conversion is Björn Ottosson's, through OKLab and linear sRGB.
 */
export function oklchToHex(lightness: number, chroma: number, hue: number): string {
  const radians = (hue * Math.PI) / 180;
  const a = chroma * Math.cos(radians);
  const b = chroma * Math.sin(radians);

  const l = (lightness + 0.3963377774 * a + 0.2158037573 * b) ** 3;
  const m = (lightness - 0.1055613458 * a - 0.0638541728 * b) ** 3;
  const s = (lightness - 0.0894841775 * a - 1.291485548 * b) ** 3;

  const linear = [
    4.0767416621 * l - 3.3077115913 * m + 0.2309699292 * s,
    -1.2684380046 * l + 2.6097574011 * m - 0.3413193965 * s,
    -0.0041960863 * l - 0.7034186147 * m + 1.707614701 * s,
  ];
  return `#${linear.map((channel) => hexByte(encode(channel))).join("")}`;
}

/** sRGB's transfer function, from linear light to the encoded value. */
function encode(linear: number): number {
  const clipped = Math.min(1, Math.max(0, linear));
  return clipped <= 0.0031308 ? 12.92 * clipped : 1.055 * clipped ** (1 / 2.4) - 0.055;
}

function hexByte(value: number): string {
  return Math.round(value * 255)
    .toString(16)
    .padStart(2, "0");
}
