// Copied into an isolated Unity project by unity-humanoid-smoke.ps1.
using System;
using System.IO;
using System.Linq;
using UnityEditor;
using UnityEngine;

public static class AxiomUnitySmoke
{
    public static void Run()
    {
        const string path = "Assets/fixture.fbx";
        var prefab = AssetDatabase.LoadAssetAtPath<GameObject>(path);
        var clip = AssetDatabase.LoadAllAssetsAtPath(path).OfType<AnimationClip>().First(c => c.name == "Walking");
        var model = UnityEngine.Object.Instantiate(prefab);
        try
        {
            var animator = model.GetComponentInChildren<Animator>();
            if (animator == null || !animator.isHuman || !clip.humanMotion)
                throw new Exception("Fixture must contain a valid Avatar and Humanoid motion");
            AxiomHumanoidExport.Export(animator, clip, 30, Path.GetFullPath("../unity-walk.axrig"));
            Debug.Log("AXIOM_UNITY_HUMANOID_EXPORT_OK " + Application.unityVersion);
        }
        finally { UnityEngine.Object.DestroyImmediate(model); }
    }
}
