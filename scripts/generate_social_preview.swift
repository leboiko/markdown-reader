#!/usr/bin/swift

import AppKit
import Foundation

let fm = FileManager.default
let cwd = URL(fileURLWithPath: fm.currentDirectoryPath)
let logoURL = cwd.appendingPathComponent("docs/assets/logo.png")
let outputURL = cwd.appendingPathComponent("docs/assets/social-preview.png")

guard let logo = NSImage(contentsOf: logoURL) else {
    fputs("failed to load logo at \(logoURL.path)\n", stderr)
    exit(1)
}

let canvasSize = NSSize(width: 1280, height: 640)
let canvas = NSImage(size: canvasSize)

canvas.lockFocus()

let background = NSColor(calibratedRed: 0.98, green: 0.97, blue: 0.95, alpha: 1.0)
background.setFill()
NSBezierPath(rect: NSRect(origin: .zero, size: canvasSize)).fill()

let cardRect = NSRect(x: 52, y: 52, width: canvasSize.width - 104, height: canvasSize.height - 104)
let cardPath = NSBezierPath(roundedRect: cardRect, xRadius: 28, yRadius: 28)
NSColor.white.setFill()
cardPath.fill()

NSColor(calibratedWhite: 0.0, alpha: 0.08).setStroke()
cardPath.lineWidth = 2
cardPath.stroke()

let logoRect = NSRect(x: 112, y: 136, width: 360, height: 368)
logo.draw(in: logoRect)

let title = "markdown-reader" as NSString
let subtitle = "Terminal markdown repo browser with tabs, search,\nMermaid, math, and tables." as NSString
let kicker = "Rust + ratatui" as NSString

let titleStyle = NSMutableParagraphStyle()
titleStyle.lineBreakMode = .byWordWrapping

let subtitleStyle = NSMutableParagraphStyle()
subtitleStyle.lineBreakMode = .byWordWrapping

let titleAttrs: [NSAttributedString.Key: Any] = [
    .font: NSFont.systemFont(ofSize: 60, weight: .bold),
    .foregroundColor: NSColor(calibratedRed: 0.15, green: 0.12, blue: 0.16, alpha: 1.0),
    .paragraphStyle: titleStyle,
]

let subtitleAttrs: [NSAttributedString.Key: Any] = [
    .font: NSFont.systemFont(ofSize: 28, weight: .medium),
    .foregroundColor: NSColor(calibratedRed: 0.34, green: 0.29, blue: 0.31, alpha: 1.0),
    .paragraphStyle: subtitleStyle,
]

let kickerAttrs: [NSAttributedString.Key: Any] = [
    .font: NSFont.systemFont(ofSize: 20, weight: .semibold),
    .foregroundColor: NSColor(calibratedRed: 0.58, green: 0.32, blue: 0.24, alpha: 1.0),
]

let textX: CGFloat = 520
title.draw(in: NSRect(x: textX, y: 360, width: 620, height: 90), withAttributes: titleAttrs)
subtitle.draw(in: NSRect(x: textX, y: 235, width: 620, height: 110), withAttributes: subtitleAttrs)
kicker.draw(in: NSRect(x: textX, y: 185, width: 240, height: 40), withAttributes: kickerAttrs)

canvas.unlockFocus()

guard
    let tiff = canvas.tiffRepresentation,
    let rep = NSBitmapImageRep(data: tiff),
    let png = rep.representation(using: .png, properties: [:])
else {
    fputs("failed to encode PNG\n", stderr)
    exit(1)
}

try png.write(to: outputURL)
print("wrote \(outputURL.path)")
