declare module 'apca-w3' {
  /**
   * APCA lightness contrast (Lc). Positive for dark text on a light
   * background, negative for light text on a dark one.
   *
   * @param textColour - sRGB hex string, e.g. `#1a1a1a`
   * @param bgColour - sRGB hex string
   */
  export function calcAPCA(textColour: string, bgColour: string): number;
}
