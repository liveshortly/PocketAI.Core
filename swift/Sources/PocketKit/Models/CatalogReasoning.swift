import Foundation

/// CORE-14: reasoning capability comes from the catalog manifest's
/// `reasoning` flag (core: `ModelManifest.reasoning`; curated entries carry it
/// in catalog.json, uncurated ones get it from `pocket_models::supports_reasoning`).
/// This only resolves a bare chat model id against an already-loaded catalog:
/// "mac:<peer>/<id>" resolves to its local id. Unknown ids → false.
public extension Array where Element == CatalogEntry {
    func supportsReasoning(modelID: String) -> Bool {
        let id = modelID.hasPrefix("mac:") ? RemoteEngine.localModelId(from: modelID) : modelID
        return first { $0.manifest.id == id }?.manifest.reasoning ?? false
    }
}
