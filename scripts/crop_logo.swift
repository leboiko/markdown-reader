#!/usr/bin/swift

import AppKit
import Foundation

let fm = FileManager.default
let cwd = URL(fileURLWithPath: fm.currentDirectoryPath)
let inputURL = cwd.appendingPathComponent("docs/assets/logo.png")

guard
    let image = NSImage(contentsOf: inputURL),
    let tiff = image.tiffRepresentation,
    let rep = NSBitmapImageRep(data: tiff)
else {
    fputs("failed to load image at \(inputURL.path)\n", stderr)
    exit(1)
}

let width = rep.pixelsWide
let height = rep.pixelsHigh
let padding = 24
let alphaThreshold: CGFloat = 0.05
let alphaThresholdByte = UInt8(alphaThreshold * 255.0)

// First pass: fully clear the faint low-alpha white veil carried by the source
// image so dark-mode backgrounds do not show a tinted rectangle around the logo.
guard let bitmapData = rep.bitmapData else {
    fputs("failed to access bitmap data\n", stderr)
    exit(1)
}
let bytesPerPixel = rep.bitsPerPixel / 8
for y in 0..<height {
    for x in 0..<width {
        let offset = y * rep.bytesPerRow + x * bytesPerPixel
        let alpha = bitmapData[offset + 3]
        if alpha < alphaThresholdByte {
            bitmapData[offset + 0] = 0
            bitmapData[offset + 1] = 0
            bitmapData[offset + 2] = 0
            bitmapData[offset + 3] = 0
        }
    }
}

var minX = width
var minY = height
var maxX = -1
var maxY = -1

for y in 0..<height {
    for x in 0..<width {
        guard let color = rep.colorAt(x: x, y: y) else { continue }
        if color.alphaComponent > alphaThreshold {
            minX = min(minX, x)
            minY = min(minY, y)
            maxX = max(maxX, x)
            maxY = max(maxY, y)
        }
    }
}

guard minX <= maxX, minY <= maxY else {
    fputs("image appears fully transparent\n", stderr)
    exit(1)
}

let cropX = max(minX - padding, 0)
let cropY = max(minY - padding, 0)
let cropWidth = min((maxX - minX + 1) + padding * 2, width - cropX)
let cropHeight = min((maxY - minY + 1) + padding * 2, height - cropY)

guard let outRep = NSBitmapImageRep(
    bitmapDataPlanes: nil,
    pixelsWide: cropWidth,
    pixelsHigh: cropHeight,
    bitsPerSample: rep.bitsPerSample,
    samplesPerPixel: rep.samplesPerPixel,
    hasAlpha: rep.hasAlpha,
    isPlanar: false,
    colorSpaceName: .deviceRGB,
    bytesPerRow: cropWidth * bytesPerPixel,
    bitsPerPixel: rep.bitsPerPixel
), let outData = outRep.bitmapData else {
    fputs("failed to allocate cropped bitmap\n", stderr)
    exit(1)
}

for y in 0..<cropHeight {
    let srcRow = (cropY + y) * rep.bytesPerRow + cropX * bytesPerPixel
    let dstRow = y * outRep.bytesPerRow
    memcpy(outData + dstRow, bitmapData + srcRow, cropWidth * bytesPerPixel)
}

guard let png = outRep.representation(using: .png, properties: [:]) else {
    fputs("failed to encode cropped PNG\n", stderr)
    exit(1)
}

try png.write(to: inputURL)
print("cropped \(inputURL.path) to \(cropWidth)x\(cropHeight)")
