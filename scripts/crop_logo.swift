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

guard minX <= maxX, minY <= maxY, let cgImage = rep.cgImage else {
    fputs("image appears fully transparent\n", stderr)
    exit(1)
}

let cropX = max(minX - padding, 0)
let cropY = max(minY - padding, 0)
let cropWidth = min((maxX - minX + 1) + padding * 2, width - cropX)
let cropHeight = min((maxY - minY + 1) + padding * 2, height - cropY)

let cropRect = CGRect(
    x: cropX,
    y: cropY,
    width: cropWidth,
    height: cropHeight
)

guard let cropped = cgImage.cropping(to: cropRect) else {
    fputs("failed to crop image\n", stderr)
    exit(1)
}

let outRep = NSBitmapImageRep(cgImage: cropped)
guard let png = outRep.representation(using: .png, properties: [:]) else {
    fputs("failed to encode cropped PNG\n", stderr)
    exit(1)
}

try png.write(to: inputURL)
print("cropped \(inputURL.path) to \(cropWidth)x\(cropHeight)")
