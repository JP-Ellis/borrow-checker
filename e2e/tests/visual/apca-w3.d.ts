declare module 'apca-w3' {
    /**
     * Calculate the APCA (Accessible Perceptual Contrast Algorithm) Lc value.
     *
     * @param textColour - sRGB hex string for the text/ink colour
     * @param bgColour - sRGB hex string for the background colour
     * @returns Lc value (positive = dark text on light bg, negative = light text on dark bg)
     */
    export function calcAPCA(textColour: string, bgColour: string): number;
}
