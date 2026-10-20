import CoreGraphics
import Foundation
// Usage: winid <owner-substring>  -> prints "id x y w h title" per on-screen window
let want = CommandLine.arguments.count > 1 ? CommandLine.arguments[1].lowercased() : ""
let list = CGWindowListCopyWindowInfo([.optionOnScreenOnly], kCGNullWindowID) as! [[String: Any]]
for w in list {
  let owner = (w[kCGWindowOwnerName as String] as? String) ?? ""
  let pid = (w[kCGWindowOwnerPID as String] as? Int) ?? 0
  if !want.isEmpty && !owner.lowercased().contains(want) { continue }
  let b = w[kCGWindowBounds as String] as! [String: Any]
  let layer = (w[kCGWindowLayer as String] as? Int) ?? 0
  print(w[kCGWindowNumber as String]!, pid, layer, b["X"]!, b["Y"]!, b["Width"]!, b["Height"]!, owner, (w[kCGWindowName as String] as? String) ?? "")
}
