import AppKit
import Foundation

enum IconError: Error {
    case bitmapCreation
    case pngEncoding
}

func renderIcon(pixelSize: Int) throws -> Data {
    guard let bitmap = NSBitmapImageRep(
        bitmapDataPlanes: nil,
        pixelsWide: pixelSize,
        pixelsHigh: pixelSize,
        bitsPerSample: 8,
        samplesPerPixel: 4,
        hasAlpha: true,
        isPlanar: false,
        colorSpaceName: .deviceRGB,
        bytesPerRow: 0,
        bitsPerPixel: 0
    ), let graphics = NSGraphicsContext(bitmapImageRep: bitmap) else {
        throw IconError.bitmapCreation
    }
    let context = graphics.cgContext
    context.clear(CGRect(x: 0, y: 0, width: pixelSize, height: pixelSize))
    let scale = CGFloat(pixelSize) / 1024
    context.scaleBy(x: scale, y: scale)
    context.translateBy(x: 0, y: 1024)
    context.scaleBy(x: 1, y: -1)
    context.setShouldAntialias(true)
    context.setFillColor(CGColor(srgbRed: 34 / 255, green: 34 / 255, blue: 34 / 255, alpha: 1))
    context.addPath(CGPath(
        roundedRect: CGRect(x: 80, y: 80, width: 864, height: 864),
        cornerWidth: 192,
        cornerHeight: 192,
        transform: nil
    ))
    context.fillPath()
    context.translateBy(x: 192, y: 192)
    context.scaleBy(x: 40, y: 40)
    context.setStrokeColor(CGColor(srgbRed: 219 / 255, green: 215 / 255, blue: 202 / 255, alpha: 1))
    context.setLineWidth(1.5)
    context.setLineCap(.round)
    context.strokeEllipse(in: CGRect(x: 2, y: 2, width: 9, height: 9))
    context.move(to: CGPoint(x: 9.75, y: 9.75))
    context.addLine(to: CGPoint(x: 13.5, y: 13.5))
    context.strokePath()
    guard let png = bitmap.representation(using: .png, properties: [:]) else {
        throw IconError.pngEncoding
    }
    return png
}

func iconChunk(type: String, content: Data) -> Data {
    var chunk = Data(type.utf8)
    let length = UInt32(content.count + 8).bigEndian
    withUnsafeBytes(of: length) { chunk.append(contentsOf: $0) }
    chunk.append(content)
    return chunk
}

guard CommandLine.arguments.count == 2 else {
    fputs("Usage: swift scripts/icon.swift <output.icns>\n", stderr)
    exit(1)
}

do {
    let output = URL(fileURLWithPath: CommandLine.arguments[1])
    let representations = [
        (16, "icp4"), (32, "ic11"),
        (32, "icp5"), (64, "ic12"),
        (128, "ic07"), (256, "ic13"),
        (256, "ic08"), (512, "ic14"),
        (512, "ic09"), (1024, "ic10"),
    ]
    var images = Data()
    try representations.forEach { size, type in
        images.append(iconChunk(type: type, content: try renderIcon(pixelSize: size)))
    }
    try iconChunk(type: "icns", content: images).write(to: output)
} catch {
    fputs("Could not generate app icon: \(error)\n", stderr)
    exit(1)
}
