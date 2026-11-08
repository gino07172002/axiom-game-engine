// Copy into a Unity project's Assets/Editor folder. No package dependency.
// Exports evaluated Avatar bone TRS, skinned geometry and a semantic profile to .axrig v1.
#if UNITY_EDITOR
using System;
using System.Collections;
using System.Collections.Generic;
using System.Globalization;
using System.IO;
using System.Linq;
using System.Text;
using UnityEditor;
using UnityEngine;
using UnityEngine.Animations;
using UnityEngine.Playables;

public sealed class AxiomHumanoidExport : EditorWindow
{
    Animator source;
    AnimationClip clip;
    int fps = 30;
    float metersPerUnit = 1f;
    [MenuItem("Tools/Axiom/Export Humanoid")]
    static void Open() { GetWindow<AxiomHumanoidExport>("Axiom Humanoid"); }
    void OnGUI()
    {
        source = (Animator)EditorGUILayout.ObjectField("Avatar Animator", source, typeof(Animator), true);
        clip = (AnimationClip)EditorGUILayout.ObjectField("Clip (optional)", clip, typeof(AnimationClip), false);
        fps = EditorGUILayout.IntSlider("Sample FPS", fps, 1, 120);
        metersPerUnit = EditorGUILayout.FloatField("Meters per scene unit", metersPerUnit);
        EditorGUILayout.HelpBox("Exports a temporary clone. Avatar must be valid Humanoid. Current reference pose is baked into the mesh; source assets are unchanged. Root motion is sampled as bone transforms, not Mecanim muscle curves.", MessageType.Info);
        using (new EditorGUI.DisabledScope(source == null))
            if (GUILayout.Button("Export .axrig"))
            {
                string path = EditorUtility.SaveFilePanel("Export Axiom rig", "", source.name + ".axrig", "axrig");
                if (path.Length != 0)
                    try { Export(source, clip, fps, path, metersPerUnit); Debug.Log("Axiom exported " + path); }
                    catch (Exception e) { Debug.LogException(e); }
            }
    }
    static object[] V(Vector3 v) { return new object[] { v.x, v.y, v.z }; }
    static object[] Q(Quaternion q) { return new object[] { q.x, q.y, q.z, q.w }; }
    static object[] V4(Vector3 v) { return new object[] { v.x, v.y, v.z, 0f }; }
    static Dictionary<string, object> D(params object[] fields)
    {
        var d = new Dictionary<string, object>();
        for (int i = 0; i < fields.Length; i += 2) d.Add((string)fields[i], fields[i + 1]);
        return d;
    }
    static object Matrix(Matrix4x4 m)
    {
        // glam serde Mat4 is a flat, column-major 16-float array.
        return Enumerable.Range(0, 16).Select(i => (object)m[i]).ToArray();
    }
    public static void Export(Animator animator, AnimationClip animation, int rate, string path, float metersPerUnit = 1f)
    {
        if (animator.avatar == null || !animator.avatar.isValid || !animator.avatar.isHuman)
            throw new InvalidOperationException("Select an Animator with a valid Humanoid Avatar.");
        if (animation != null && animation.length > 600) throw new InvalidOperationException("Clip exceeds 600 seconds.");
        if (rate < 1 || rate > 120 || !(metersPerUnit > 0.000001f && metersPerUnit <= 1000f))
            throw new ArgumentOutOfRangeException("Sampling rate or scene unit scale");
        string directory = Path.GetDirectoryName(Path.GetFullPath(path));
        Directory.CreateDirectory(directory);
        string textureDirectory = Path.GetFileNameWithoutExtension(path) + "_textures";
        Directory.CreateDirectory(Path.Combine(directory, textureDirectory));
        GameObject clone = Instantiate(animator.gameObject);
        clone.name = animator.gameObject.name;
        clone.hideFlags = HideFlags.HideAndDontSave;
        PlayableGraph graph = default(PlayableGraph);
        try
        {
            Animator rig = clone.GetComponent<Animator>();
            rig.cullingMode = AnimatorCullingMode.AlwaysAnimate;
            rig.applyRootMotion = false;
            rig.Rebind(); rig.Update(0);
            Transform root = clone.transform;
            root.position = Vector3.zero; root.rotation = Quaternion.identity; root.localScale = Vector3.one;
            Transform[] nodes = clone.GetComponentsInChildren<Transform>(true);
            if (nodes.Length > 1024) throw new InvalidOperationException("More than 1024 rig nodes. Export a reduced skeleton.");
            var slots = nodes.Select((n, i) => new { n, i }).ToDictionary(x => x.n, x => x.i);
            Func<Transform, string> id = t => t == root ? "root" : "root/" + AnimationUtility.CalculateTransformPath(t, root);
            if (nodes.Select(id).Distinct().Count() != nodes.Length) throw new InvalidOperationException("Duplicate sibling names produce ambiguous paths.");
            var mapping = new Dictionary<string, object>();
            foreach (HumanBodyBones role in Enum.GetValues(typeof(HumanBodyBones)))
            {
                if (role == HumanBodyBones.LastBone) continue;
                Transform bone = rig.GetBoneTransform(role);
                if (bone != null) mapping.Add(role.ToString(), id(bone));
            }
            var bones = new List<object>();
            foreach (Transform t in nodes)
                bones.Add(D("id", id(t), "aliases", new object[] { t.name }, "parent", t == root ? null : (object)slots[t.parent],
                    "rest", D("translation", V(t.localPosition), "rotation", Q(t.localRotation), "scale", V(t.localScale)),
                    "inverse_bind", Matrix((root.worldToLocalMatrix * t.localToWorldMatrix).inverse)));
            var vertices = new List<object>(); var indices = new List<object>(); var materials = new List<object>();
            var texturePaths = new List<object>(); var imageSlots = new Dictionary<Texture, int>();
            foreach (SkinnedMeshRenderer renderer in clone.GetComponentsInChildren<SkinnedMeshRenderer>(true))
            {
                Mesh original = renderer.sharedMesh;
                if (original == null) continue;
                var influences = original.GetBonesPerVertex();
                try { for (int v = 0; v < influences.Length; v++) if (influences[v] > 4) throw new InvalidOperationException("Reduce mesh skinning to four influences before export."); }
                finally { influences.Dispose(); }
                var mesh = new Mesh();
                try
                {
                    renderer.BakeMesh(mesh);
                    Vector3[] positions = mesh.vertices, normals = mesh.normals;
                    Vector2[] uv = mesh.uv; BoneWeight[] weights = original.boneWeights;
                    if (weights.Length != positions.Length) throw new InvalidOperationException("Mesh needs legacy four-influence bone weights; reduce influences before export.");
                    Matrix4x4 model = root.worldToLocalMatrix * renderer.localToWorldMatrix;
                    Matrix4x4 normalMatrix = model.inverse.transpose;
                    int firstVertex = vertices.Count;
                    for (int i = 0; i < positions.Length; i++)
                    {
                        BoneWeight w = weights[i]; int[] joints = { w.boneIndex0, w.boneIndex1, w.boneIndex2, w.boneIndex3 };
                        float[] values = { w.weight0, w.weight1, w.weight2, w.weight3 };
                        object[] mapped = joints.Select((j, k) => (object)(values[k] == 0 ? 0 : slots[renderer.bones[j]])).ToArray();
                        Vector3 n = normals.Length == positions.Length ? normals[i] : Vector3.up;
                        vertices.Add(D("position", V(model.MultiplyPoint3x4(positions[i])), "normal", V(normalMatrix.MultiplyVector(n).normalized),
                            "uv", new object[] { uv.Length == positions.Length ? uv[i].x : 0f, uv.Length == positions.Length ? 1f - uv[i].y : 0f },
                            "joints", mapped, "weights", values, "sdef", null, "dual_quaternion", false));
                    }
                    for (int part = 0; part < original.subMeshCount; part++)
                    {
                        if (original.GetTopology(part) != MeshTopology.Triangles) throw new InvalidOperationException("Only triangle submeshes are supported.");
                        int first = indices.Count;
                        foreach (int index in original.GetTriangles(part)) indices.Add(index + firstVertex);
                        Material material = part < renderer.sharedMaterials.Length ? renderer.sharedMaterials[part] : null;
                        Color color = material != null && material.HasProperty("_BaseColor") ? material.GetColor("_BaseColor") : material != null && material.HasProperty("_Color") ? material.color : Color.white;
                        Texture texture = material == null ? null : material.mainTexture;
                        object textureIndex = null;
                        if (texture != null)
                        {
                            int slot;
                            if (!imageSlots.TryGetValue(texture, out slot))
                            {
                                slot = texturePaths.Count;
                                imageSlots.Add(texture, slot);
                                string file = slot + ".png";
                                SaveTexture(texture, Path.Combine(directory, textureDirectory, file));
                                texturePaths.Add(textureDirectory + "/" + file);
                            }
                            textureIndex = slot;
                        }
                        object alpha = color.a < .999f ? (object)"blend" : D("mask", .1f);
                        materials.Add(D("id", id(renderer.transform) + "/" + part, "first_index", first, "index_count", indices.Count - first,
                            "color", new object[] { color.r, color.g, color.b, color.a }, "texture", textureIndex, "double_sided", true, "alpha", alpha));
                    }
                }
                finally { DestroyImmediate(mesh); }
            }
            var clips = new List<object>();
            if (animation != null)
            {
                int count = Mathf.CeilToInt(animation.length * rate) + 1;
                if ((long)count * nodes.Length * 3 > 2000000) throw new InvalidOperationException("Animation key budget exceeded. Reduce FPS or duration.");
                graph = PlayableGraph.Create("Axiom export sampling");
                graph.SetTimeUpdateMode(DirectorUpdateMode.Manual);
                var playable = AnimationClipPlayable.Create(graph, animation);
                playable.SetApplyFootIK(false); playable.SetApplyPlayableIK(false);
                var output = AnimationPlayableOutput.Create(graph, "Avatar", rig);
                output.SetSourcePlayable(playable); graph.Play();
                var positions = nodes.Select(n => new List<object>()).ToArray();
                var rotations = nodes.Select(n => new List<object>()).ToArray();
                var scales = nodes.Select(n => new List<object>()).ToArray();
                var times = new List<object>();
                for (int frame = 0; frame < count; frame++)
                {
                    float time = Mathf.Min(frame / (float)rate, animation.length);
                    playable.SetTime(time); graph.Evaluate(0);
                    times.Add(time);
                    for (int i = 0; i < nodes.Length; i++)
                    {
                        positions[i].Add(V4(nodes[i].localPosition)); rotations[i].Add(Q(nodes[i].localRotation)); scales[i].Add(V4(nodes[i].localScale));
                    }
                }
                var tracks = new List<object>();
                for (int i = 0; i < nodes.Length; i++)
                {
                    tracks.Add(D("bone", id(nodes[i]), "property", "translation", "interpolation", "linear", "times", times, "values", positions[i]));
                    tracks.Add(D("bone", id(nodes[i]), "property", "rotation", "interpolation", "linear", "times", times, "values", rotations[i]));
                    tracks.Add(D("bone", id(nodes[i]), "property", "scale", "interpolation", "linear", "times", times, "values", scales[i]));
                }
                clips.Add(D("id", animation.name, "duration", animation.length, "tracks", tracks));
            }
            var bundle = D("version", 1, "coordinates", D("basis", new object[] { new object[] { 1, 0, 0 }, new object[] { 0, 1, 0 }, new object[] { 0, 0, -1 } }, "meters_per_unit", metersPerUnit),
                "skeleton", D("bones", bones), "vertices", vertices, "indices", indices, "materials", materials, "textures", texturePaths, "clips", clips,
                "humanoid", D("mapping", mapping, "reference", D()));
            using (var writer = new StreamWriter(path, false, new UTF8Encoding(false))) WriteJson(writer, bundle);
        }
        finally { if (graph.IsValid()) graph.Destroy(); DestroyImmediate(clone); }
    }
    static void SaveTexture(Texture source, string path)
    {
        if (source.width > 4096 || source.height > 4096) throw new InvalidOperationException("Texture exceeds 4096 pixels; reduce it before export.");
        var target = RenderTexture.GetTemporary(source.width, source.height, 0, RenderTextureFormat.ARGB32, RenderTextureReadWrite.sRGB);
        RenderTexture previous = RenderTexture.active;
        Texture2D texture = null;
        try
        {
            Graphics.Blit(source, target); RenderTexture.active = target;
            texture = new Texture2D(source.width, source.height, TextureFormat.RGBA32, false);
            texture.ReadPixels(new Rect(0, 0, source.width, source.height), 0, 0); texture.Apply();
            File.WriteAllBytes(path, texture.EncodeToPNG());
        }
        finally { RenderTexture.active = previous; RenderTexture.ReleaseTemporary(target); if (texture != null) DestroyImmediate(texture); }
    }
    static void WriteJson(TextWriter writer, object value)
    {
        if (value == null) { writer.Write("null"); return; }
        if (value is string) { writer.Write('"'); foreach (char c in (string)value) { if (c == '"' || c == '\\') { writer.Write('\\'); writer.Write(c); } else if (c < 32) writer.Write("\\u" + ((int)c).ToString("x4")); else writer.Write(c); } writer.Write('"'); return; }
        if (value is bool) { writer.Write((bool)value ? "true" : "false"); return; }
        if (value is IDictionary) { writer.Write('{'); bool first = true; foreach (DictionaryEntry entry in (IDictionary)value) { if (!first) writer.Write(','); first = false; WriteJson(writer, entry.Key); writer.Write(':'); WriteJson(writer, entry.Value); } writer.Write('}'); return; }
        if (value is IEnumerable) { writer.Write('['); bool first = true; foreach (object item in (IEnumerable)value) { if (!first) writer.Write(','); first = false; WriteJson(writer, item); } writer.Write(']'); return; }
        string number = Convert.ToString(value, CultureInfo.InvariantCulture);
        if (number == "NaN" || number.Contains("Infinity")) throw new InvalidOperationException("Non-finite export value.");
        writer.Write(number);
    }
}
#endif
