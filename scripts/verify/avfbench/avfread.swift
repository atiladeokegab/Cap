// Reads every video sample of a file with AVAssetReader (NV12 output at the
// native size, as Cap's decoder asks for) and prints per-sample timing.
// Usage: avfread <file.mp4> <label>
import AVFoundation
import Foundation

let args = CommandLine.arguments
let url = URL(fileURLWithPath: args[1])
let label = args.count > 2 ? args[2] : "run"
let asset = AVURLAsset(url: url)
let sema = DispatchSemaphore(value: 0)
var track: AVAssetTrack?
asset.loadTracks(withMediaType: .video) { tracks, _ in
    track = tracks?.first
    sema.signal()
}
sema.wait()
guard let track else { fatalError("no video track") }
let size = track.naturalSize
let reader = try AVAssetReader(asset: asset)
let output = AVAssetReaderTrackOutput(track: track, outputSettings: [
    kCVPixelBufferPixelFormatTypeKey as String: kCVPixelFormatType_420YpCbCr8BiPlanarVideoRange,
    kCVPixelBufferWidthKey as String: Int(size.width),
    kCVPixelBufferHeightKey as String: Int(size.height),
])
output.alwaysCopiesSampleData = false
reader.add(output)
reader.startReading()
var times: [Double] = []
let start = DispatchTime.now().uptimeNanoseconds
while true {
    let t0 = DispatchTime.now().uptimeNanoseconds
    guard let sample = output.copyNextSampleBuffer() else { break }
    let ms = Double(DispatchTime.now().uptimeNanoseconds - t0) / 1e6
    if CMSampleBufferGetImageBuffer(sample) != nil { times.append(ms) }
}
let total = Double(DispatchTime.now().uptimeNanoseconds - start) / 1e6
let sorted = times.sorted()
func pct(_ p: Double) -> Double { sorted.isEmpty ? 0 : sorted[min(sorted.count - 1, Int(Double(sorted.count) * p))] }
let avg = times.reduce(0, +) / Double(max(times.count, 1))
print(String(format: "%@ %dx%d frames=%d total=%.0fms avg=%.2fms p50=%.2fms p99=%.2fms max=%.2fms >=20ms=%d",
             label, Int(size.width), Int(size.height), times.count, total, avg, pct(0.5), pct(0.99), sorted.last ?? 0,
             times.filter { $0 >= 20 }.count))
