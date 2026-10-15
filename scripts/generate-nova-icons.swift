#!/usr/bin/env swift
// Resize the generated Nova master and apply platform icon framing.
// Run from the repository root: swift scripts/generate-nova-icons.swift
import Foundation
import CoreGraphics
import ImageIO
import UniformTypeIdentifiers

let root = URL(fileURLWithPath: FileManager.default.currentDirectoryPath)
let input = root.appendingPathComponent("dist/zeron-nova-source.png")
guard let source = CGImageSourceCreateWithURL(input as CFURL, nil),
      let image = CGImageSourceCreateImageAtIndex(source, 0, nil),
      let colorSpace = CGColorSpace(name: CGColorSpace.sRGB) else {
    fatalError("Cannot read the Nova source artwork")
}

func write(_ path: String, size: Int, macOS: Bool = false) throws {
    let alpha = macOS ? CGImageAlphaInfo.premultipliedLast : .noneSkipLast
    guard let context = CGContext(data: nil, width: size, height: size,
        bitsPerComponent: 8, bytesPerRow: size * 4, space: colorSpace,
        bitmapInfo: alpha.rawValue) else { fatalError("Cannot create icon bitmap") }
    context.interpolationQuality = .high
    var bounds = CGRect(x: 0, y: 0, width: size, height: size)
    if macOS {
        bounds = bounds.insetBy(dx: 100, dy: 100)
        let shape = CGPath(roundedRect: bounds, cornerWidth: 184,
            cornerHeight: 184, transform: nil)
        context.setShadow(offset: CGSize(width: 0, height: -10), blur: 18,
            color: CGColor(gray: 0, alpha: 0.3))
        context.setFillColor(CGColor(red: 0.01, green: 0.04, blue: 0.12, alpha: 1))
        context.addPath(shape)
        context.fillPath()
        context.setShadow(offset: .zero, blur: 0, color: nil)
        context.addPath(shape)
        context.clip()
    }
    context.draw(image, in: bounds)
    let output = root.appendingPathComponent(path)
    try FileManager.default.createDirectory(at: output.deletingLastPathComponent(),
        withIntermediateDirectories: true)
    guard let rendered = context.makeImage(),
          let destination = CGImageDestinationCreateWithURL(output as CFURL,
            UTType.png.identifier as CFString, 1, nil) else {
        fatalError("Cannot write \(path)")
    }
    CGImageDestinationAddImage(destination, rendered, nil)
    guard CGImageDestinationFinalize(destination) else { fatalError("Cannot save \(path)") }
    print("Wrote \(path) (\(size) × \(size))")
}

try write("dist/zeron.png", size: 1024)
try write("crates/ui/assets/brand/zeron-nova.png", size: 256)
try write("dist/macos/icon-1024.png", size: 1024, macOS: true)
try write("apps/ios/Zeron/Assets.xcassets/AppIcon.appiconset/AppIcon1024.png", size: 1024)
try write("apps/ios/Zeron/Assets.xcassets/NovaLogo.imageset/nova-logo.png", size: 256)
try write("apps/android/app/src/main/res/mipmap-xxxhdpi/ic_launcher_foreground.png", size: 512)
