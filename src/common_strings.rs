//! The table of common strings: type and field names that a type tree names by offset into this
//! buffer (a name with the top bit set in its offset) instead of storing them.
//!
//! Offsets are byte positions in [`BUFFER`], whose strings are NUL-terminated and follow one another.
//! The 101 entries are the same, at the same offsets, in two separately maintained public tables, which
//! were compared entry by entry:
//!
//! * **rabex**, git revision `56dd8a4d8d5048b6ee7664eadc89d98343c66002`, `src/commonstring.rs`
//!   (generated there from the AssetRipper TPK type tree database; it has 113 entries);
//! * **AssetStudio**, revision `d158e864b556b5970709c2a52e47944d53aa98a2`,
//!   `AssetStudio/CommonString.cs` (hand written; 109 entries).
//!
//! The table ends at `Type*` (offset 1051), though both sources go on (up to `Hash128` in
//! AssetStudio, `LoadableReference` in rabex): the game's bundles (Unity 2018.4 and 2019.4) do not
//! use anything past `Type*` and store the later names, such as `m_PrefabAsset`, in each tree's own
//! string buffer. An offset that is not the start of one of these entries is an error ("unknown
//! common string at offset N"), never a guess. `docs/FORMATS.md` says which 64 of the entries the
//! game's bundles use.

/// The strings, in order, each followed by a NUL.
pub(crate) const BUFFER: &str = concat!(
    "AABB\0",                       // 0
    "AnimationClip\0",              // 5
    "AnimationCurve\0",             // 19
    "AnimationState\0",             // 34
    "Array\0",                      // 49
    "Base\0",                       // 55
    "BitField\0",                   // 60
    "bitset\0",                     // 69
    "bool\0",                       // 76
    "char\0",                       // 81
    "ColorRGBA\0",                  // 86
    "Component\0",                  // 96
    "data\0",                       // 106
    "deque\0",                      // 111
    "double\0",                     // 117
    "dynamic_array\0",              // 124
    "FastPropertyName\0",           // 138
    "first\0",                      // 155
    "float\0",                      // 161
    "Font\0",                       // 167
    "GameObject\0",                 // 172
    "Generic Mono\0",               // 183
    "GradientNEW\0",                // 196
    "GUID\0",                       // 208
    "GUIStyle\0",                   // 213
    "int\0",                        // 222
    "list\0",                       // 226
    "long long\0",                  // 231
    "map\0",                        // 241
    "Matrix4x4f\0",                 // 245
    "MdFour\0",                     // 256
    "MonoBehaviour\0",              // 263
    "MonoScript\0",                 // 277
    "m_ByteSize\0",                 // 288
    "m_Curve\0",                    // 299
    "m_EditorClassIdentifier\0",    // 307
    "m_EditorHideFlags\0",          // 331
    "m_Enabled\0",                  // 349
    "m_ExtensionPtr\0",             // 359
    "m_GameObject\0",               // 374
    "m_Index\0",                    // 387
    "m_IsArray\0",                  // 395
    "m_IsStatic\0",                 // 405
    "m_MetaFlag\0",                 // 416
    "m_Name\0",                     // 427
    "m_ObjectHideFlags\0",          // 434
    "m_PrefabInternal\0",           // 452
    "m_PrefabParentObject\0",       // 469
    "m_Script\0",                   // 490
    "m_StaticEditorFlags\0",        // 499
    "m_Type\0",                     // 519
    "m_Version\0",                  // 526
    "Object\0",                     // 536
    "pair\0",                       // 543
    "PPtr<Component>\0",            // 548
    "PPtr<GameObject>\0",           // 564
    "PPtr<Material>\0",             // 581
    "PPtr<MonoBehaviour>\0",        // 596
    "PPtr<MonoScript>\0",           // 616
    "PPtr<Object>\0",               // 633
    "PPtr<Prefab>\0",               // 646
    "PPtr<Sprite>\0",               // 659
    "PPtr<TextAsset>\0",            // 672
    "PPtr<Texture>\0",              // 688
    "PPtr<Texture2D>\0",            // 702
    "PPtr<Transform>\0",            // 718
    "Prefab\0",                     // 734
    "Quaternionf\0",                // 741
    "Rectf\0",                      // 753
    "RectInt\0",                    // 759
    "RectOffset\0",                 // 767
    "second\0",                     // 778
    "set\0",                        // 785
    "short\0",                      // 789
    "size\0",                       // 795
    "SInt16\0",                     // 800
    "SInt32\0",                     // 807
    "SInt64\0",                     // 814
    "SInt8\0",                      // 821
    "staticvector\0",               // 827
    "string\0",                     // 840
    "TextAsset\0",                  // 847
    "TextMesh\0",                   // 857
    "Texture\0",                    // 866
    "Texture2D\0",                  // 874
    "Transform\0",                  // 884
    "TypelessData\0",               // 894
    "UInt16\0",                     // 907
    "UInt32\0",                     // 914
    "UInt64\0",                     // 921
    "UInt8\0",                      // 928
    "unsigned int\0",               // 934
    "unsigned long long\0",         // 947
    "unsigned short\0",             // 966
    "vector\0",                     // 981
    "Vector2f\0",                   // 988
    "Vector3f\0",                   // 997
    "Vector4f\0",                   // 1006
    "m_ScriptingClassIdentifier\0", // 1015
    "Gradient\0",                   // 1042
    "Type*\0",                      // 1051
);
