import XCTest
import SwiftUI
import AppKit
import CoreGraphics
import ImageIO
@testable import PocketUI

/// Snapshot test variants required by QA-02.
public enum PocketSnapshotVariant: String, CaseIterable {
    case light
    case dark
    case ax3
}

/// Container that fixes the size and provides a deterministic canvas background for snapshots.
public struct PocketSnapshotContainer<Content: View>: View {
    let content: Content
    let size: CGSize?

    public init(content: Content, size: CGSize? = nil) {
        self.content = content
        self.size = size
    }

    public var body: some View {
        Group {
            if let size {
                VStack(alignment: .leading, spacing: 0) {
                    content
                }
                .frame(width: size.width, height: size.height, alignment: .leading)
            } else {
                VStack(alignment: .leading, spacing: 0) {
                    content
                }
                .padding(PocketSpacing.s16)
                .frame(width: 390, alignment: .leading)
            }
        }
        .background(PocketColor.bgCanvas)
    }
}

/// Renders a SwiftUI view headlessly via ImageRenderer across 3 variants (light, dark, AX3)
/// and compares against or records reference PNG snapshots.
///
/// Tolerance:
/// - Per-channel color delta tolerance: 3 / 255 (~1.2%) for antialiasing & color-space quantization differences.
/// - Maximum allowed different pixels: 1.0% (0.01) of total pixels.
@MainActor
public func assertPocketSnapshot<V: View>(
    _ view: V,
    name: String,
    fixedSize: CGSize? = nil,
    file: StaticString = #filePath,
    line: UInt = #line
) {
    let isRecordMode = ProcessInfo.processInfo.environment["POCKET_SNAPSHOT_RECORD"] == "1"
    let callerURL = URL(fileURLWithPath: "\(file)")
    let snapshotsDir = callerURL.deletingLastPathComponent().appendingPathComponent("__Snapshots__")

    for variant in PocketSnapshotVariant.allCases {
        let variantName = "\(name).\(variant.rawValue)"
        let snapshotURL = snapshotsDir.appendingPathComponent("\(variantName).png")

        let container = PocketSnapshotContainer(content: view, size: fixedSize)
        let renderedImage: CGImage?
        switch variant {
        case .light:
            renderedImage = renderView(
                container.environment(\.colorScheme, .light),
                file: file,
                line: line
            )
        case .dark:
            renderedImage = renderView(
                container.environment(\.colorScheme, .dark),
                file: file,
                line: line
            )
        case .ax3:
            renderedImage = renderView(
                container
                    .environment(\.colorScheme, .light)
                    .environment(\.dynamicTypeSize, .accessibility3)
                    .environment(\.sizeCategory, .accessibilityExtraExtraExtraLarge),
                file: file,
                line: line
            )
        }

        guard let renderedImage else {
            XCTFail("Failed to render snapshot for '\(variantName)'", file: file, line: line)
            continue
        }

        if isRecordMode {
            recordSnapshot(image: renderedImage, to: snapshotURL, variantName: variantName, file: file, line: line)
        } else {
            compareSnapshot(rendered: renderedImage, against: snapshotURL, variantName: variantName, file: file, line: line)
        }
    }
}

@MainActor
private func renderView<V: View>(_ view: V, file: StaticString, line: UInt) -> CGImage? {
    let renderer = ImageRenderer(content: view)
    renderer.scale = 2.0
    guard let cgImage = renderer.cgImage else {
        XCTFail("ImageRenderer failed to produce CGImage", file: file, line: line)
        return nil
    }
    return cgImage
}

private func recordSnapshot(
    image: CGImage,
    to url: URL,
    variantName: String,
    file: StaticString,
    line: UInt
) {
    let dir = url.deletingLastPathComponent()
    do {
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        let rep = NSBitmapImageRep(cgImage: image)
        guard let pngData = rep.representation(using: .png, properties: [:]) else {
            XCTFail("Failed to encode PNG for '\(variantName)'", file: file, line: line)
            return
        }
        try pngData.write(to: url)
    } catch {
        XCTFail("Failed to write snapshot to \(url.path): \(error)", file: file, line: line)
    }
}

private func compareSnapshot(
    rendered: CGImage,
    against url: URL,
    variantName: String,
    file: StaticString,
    line: UInt
) {
    guard FileManager.default.fileExists(atPath: url.path) else {
        XCTFail(
            "No reference snapshot found at \(url.path). Run with POCKET_SNAPSHOT_RECORD=1 to record reference images.",
            file: file,
            line: line
        )
        return
    }

    guard let imageSource = CGImageSourceCreateWithURL(url as CFURL, nil),
          let refImage = CGImageSourceCreateImageAtIndex(imageSource, 0, nil) else {
        XCTFail("Failed to load reference image at \(url.path)", file: file, line: line)
        return
    }

    guard rendered.width == refImage.width, rendered.height == refImage.height else {
        XCTFail(
            "Snapshot '\(variantName)' dimension mismatch: reference is \(refImage.width)x\(refImage.height), rendered is \(rendered.width)x\(rendered.height)",
            file: file,
            line: line
        )
        return
    }

    let width = rendered.width
    let height = rendered.height
    let bytesPerPixel = 4
    let bytesPerRow = width * bytesPerPixel
    let colorSpace = CGColorSpace(name: CGColorSpace.sRGB)!
    let bitmapInfo = CGImageAlphaInfo.premultipliedLast.rawValue | CGBitmapInfo.byteOrder32Big.rawValue

    var renBuffer = [UInt8](repeating: 0, count: height * bytesPerRow)
    var refBuffer = [UInt8](repeating: 0, count: height * bytesPerRow)

    guard let renContext = CGContext(
        data: &renBuffer,
        width: width,
        height: height,
        bitsPerComponent: 8,
        bytesPerRow: bytesPerRow,
        space: colorSpace,
        bitmapInfo: bitmapInfo
    ), let refContext = CGContext(
        data: &refBuffer,
        width: width,
        height: height,
        bitsPerComponent: 8,
        bytesPerRow: bytesPerRow,
        space: colorSpace,
        bitmapInfo: bitmapInfo
    ) else {
        XCTFail("Failed to create CGContext for pixel comparison of '\(variantName)'", file: file, line: line)
        return
    }

    renContext.draw(rendered, in: CGRect(x: 0, y: 0, width: width, height: height))
    refContext.draw(refImage, in: CGRect(x: 0, y: 0, width: width, height: height))

    let totalPixels = width * height
    var differentPixels = 0

    // Tolerance settings:
    // - perChannelTolerance: 3/255 accounts for antialiasing & subpixel rounding differences across minor OS builds
    // - maxDiffFraction: 0.01 (1.0%) allows <= 1% differing pixels
    let perChannelTolerance: Int = 3
    let maxAllowedDiffFraction: Double = 0.01

    for i in 0..<totalPixels {
        let offset = i * bytesPerPixel
        let rDiff = abs(Int(renBuffer[offset]) - Int(refBuffer[offset]))
        let gDiff = abs(Int(renBuffer[offset + 1]) - Int(refBuffer[offset + 1]))
        let bDiff = abs(Int(renBuffer[offset + 2]) - Int(refBuffer[offset + 2]))
        let aDiff = abs(Int(renBuffer[offset + 3]) - Int(refBuffer[offset + 3]))

        if max(rDiff, gDiff, bDiff, aDiff) > perChannelTolerance {
            differentPixels += 1
        }
    }

    let diffFraction = Double(differentPixels) / Double(totalPixels)
    if diffFraction > maxAllowedDiffFraction {
        let percentString = String(format: "%.2f", diffFraction * 100)
        let allowedString = String(format: "%.2f", maxAllowedDiffFraction * 100)
        XCTFail(
            "Snapshot '\(variantName)' failed: \(differentPixels)/\(totalPixels) pixels differ (\(percentString)% > allowed \(allowedString)%) beyond per-channel tolerance of \(perChannelTolerance)/255.",
            file: file,
            line: line
        )
    }
}
