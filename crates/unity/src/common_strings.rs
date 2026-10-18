//! Unity's built-in type-tree string table (engine metadata, the same for
//! every 2022.3 build): type-tree blobs refer to these strings by byte offset
//! (high bit set) into their NUL-separated concatenation. Generated from
//! UnityPy's TPK database.

/// The strings in offset order.
pub const COMMON_STRINGS: &[&str] = &[
    "AABB",
    "AnimationClip",
    "AnimationCurve",
    "AnimationState",
    "Array",
    "Base",
    "BitField",
    "bitset",
    "bool",
    "char",
    "ColorRGBA",
    "Component",
    "data",
    "deque",
    "double",
    "dynamic_array",
    "FastPropertyName",
    "first",
    "float",
    "Font",
    "GameObject",
    "Generic Mono",
    "GradientNEW",
    "GUID",
    "GUIStyle",
    "int",
    "list",
    "long long",
    "map",
    "Matrix4x4f",
    "MdFour",
    "MonoBehaviour",
    "MonoScript",
    "m_ByteSize",
    "m_Curve",
    "m_EditorClassIdentifier",
    "m_EditorHideFlags",
    "m_Enabled",
    "m_ExtensionPtr",
    "m_GameObject",
    "m_Index",
    "m_IsArray",
    "m_IsStatic",
    "m_MetaFlag",
    "m_Name",
    "m_ObjectHideFlags",
    "m_PrefabInternal",
    "m_PrefabParentObject",
    "m_Script",
    "m_StaticEditorFlags",
    "m_Type",
    "m_Version",
    "Object",
    "pair",
    "PPtr<Component>",
    "PPtr<GameObject>",
    "PPtr<Material>",
    "PPtr<MonoBehaviour>",
    "PPtr<MonoScript>",
    "PPtr<Object>",
    "PPtr<Prefab>",
    "PPtr<Sprite>",
    "PPtr<TextAsset>",
    "PPtr<Texture>",
    "PPtr<Texture2D>",
    "PPtr<Transform>",
    "Prefab",
    "Quaternionf",
    "Rectf",
    "RectInt",
    "RectOffset",
    "second",
    "set",
    "short",
    "size",
    "SInt16",
    "SInt32",
    "SInt64",
    "SInt8",
    "staticvector",
    "string",
    "TextAsset",
    "TextMesh",
    "Texture",
    "Texture2D",
    "Transform",
    "TypelessData",
    "UInt16",
    "UInt32",
    "UInt64",
    "UInt8",
    "unsigned int",
    "unsigned long long",
    "unsigned short",
    "vector",
    "Vector2f",
    "Vector3f",
    "Vector4f",
    "m_ScriptingClassIdentifier",
    "Gradient",
    "Type*",
    "int2_storage",
    "int3_storage",
    "BoundsInt",
    "m_CorrespondingSourceObject",
    "m_PrefabInstance",
    "m_PrefabAsset",
    "FileSize",
    "Hash128",
    "RenderingLayerMask",
    "fixed_array",
    "EntityId",
    "LoadableObjectId",
    "LoadableSceneId",
];

/// String at byte offset `off` of the concatenated table.
pub fn common_string(off: u32) -> Option<&'static str> {
    let mut pos = 0u32;
    for s in COMMON_STRINGS {
        if pos == off {
            return Some(s);
        }
        pos += s.len() as u32 + 1;
    }
    None
}

#[cfg(test)]
mod tests {
    #[test]
    fn offsets() {
        assert_eq!(super::common_string(0), Some("AABB"));
        assert_eq!(super::common_string(5), Some("AnimationClip"));
        assert_eq!(super::common_string(1), None);
    }
}
