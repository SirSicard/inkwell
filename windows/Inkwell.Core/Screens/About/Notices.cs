// The notices of everything the Windows app ships that is not Inkwell's own: the code compiled into
// it or copied beside it, and the model weights it downloads. Settings > About shows them. The
// Windows counterpart of the Mac's mac/Sources/Inkwell/Screens/Notices.swift.
//
// The third-party Rust crates linked into the core (ink_ffi.dll) are not here: their list is
// generated from cargo's resolution of the Windows release build (RustNotices, RustNotices.g.cs, by
// `cargo run -p ink-ffi --bin ink-notices -- --windows`), and About shows it after these.
//
// What differs from the Mac's list: no AudioCap (the Mac's process tap), FluidAudio, its copy of
// fastcluster, VBx or Sparkle (Mac only); wasapi-rs (the Windows capture's process loopback),
// sherpa-onnx, ONNX Runtime and the code compiled into sherpa-onnx's library (Windows' Parakeet),
// the Windows App SDK, WebView2, WinUIEx, the .NET runtime, the Windows SDK's .NET projection,
// C#/WinRT and Velopack (the installer and updates) in their place. Their texts are in
// NoticeTexts.cs, copied from the packages the Windows build restores (sherpa-onnx's is the shared
// Apache License). sherpa-onnx's library compiles in its own copy of fastcluster (hclust-cpp's),
// whose notice is here, hclust-cpp's own.
//
// The texts shared with the Mac (llama.cpp down to webgl-noise, the Apache License, Silero's) are
// the Mac's, copied verbatim from Notices.swift; NoticesTests holds them equal to it. They are the
// components' own licence files except where Notices.swift says otherwise, and the composed ones
// (`Composed`) are listed with their upstream check in mac/composed-notices.txt; the Windows-only
// composed ones (WinUIEx, the Windows SDK projection, ONNX Runtime and the code compiled into
// sherpa-onnx's library, whose packages carry no licence text) in composed-notices.txt beside this
// file.
//
// No notice ships as a placeholder: `Pending` names where a text must come from while it is one,
// and NoticesTests holds the list of pending notices empty.

namespace Inkwell.Core.Screens;

/// <summary>A component's notice.</summary>
/// <param name="Id">Its id, as the Mac's where it has one.</param>
/// <param name="Name">Its name and authors.</param>
/// <param name="Role">What it does in Inkwell.</param>
/// <param name="Licence">Its licence, by name.</param>
/// <param name="Text">The notice and licence text.</param>
public sealed record ThirdPartyNotice(string Id, string Name, string Role, string Licence, string Text)
{
    /// <summary>
    /// Composed from a licence's standard text and the component's copyright line, because its own
    /// licence file was not on hand: mac/composed-notices.txt must list it.
    /// </summary>
    public bool Composed { get; init; }

    /// <summary>
    /// Where the text must come from, while <see cref="Text"/> is a placeholder; null once the text
    /// is the component's own.
    /// </summary>
    public string? Pending { get; init; }

    /// <summary>About's row: the component and its licence.</summary>
    public string Title => $"{Name} ({Licence})";
}

/// <summary>Model weights the app downloads, credited by name, author and licence.</summary>
/// <param name="Notice">The licence notice to show, where the licence asks for one.</param>
public sealed record ModelCredit(string Id, string Name, string Author, string Licence, string Use, string? Notice)
{
    /// <summary>The notice is composed, as <see cref="ThirdPartyNotice.Composed"/>.</summary>
    public bool Composed { get; init; }

    /// <summary>About's row: the model, its author and its licence.</summary>
    public string Title => $"{Name}, by {Author} ({Licence})";
}

/// <summary>
/// A Rust crate linked into the core, with its licence files. The list, <c>RustNotices.Crates</c>,
/// is generated (RustNotices.g.cs) from cargo's resolution of the Windows release build, so it
/// follows Cargo.lock; the core's tests and RustNoticesTests fail while it was made from another
/// lock.
/// </summary>
/// <param name="Licence">Its licence as it publishes it (an SPDX expression, such as "MIT OR Apache-2.0").</param>
/// <param name="Shown">The licence its texts below are, where it offers a choice ("MIT").</param>
/// <param name="Text">
/// Its licence files, each under a "--- name ---" line, with a bracketed note where Inkwell supplied
/// a text or a copyright line the package lacks.
/// </param>
public sealed record RustCrateNotice(string Name, string Version, string Licence, string Shown, string Text)
{
    /// <summary>The crate and its version.</summary>
    public string Id => $"{Name} {Version}";

    /// <summary>About's row: the crate and its version.</summary>
    public string Title => Id;

    /// <summary>About's row: its licence, and which one the text is where it offers a choice.</summary>
    public string Detail => Shown == Licence ? Licence : $"{Licence}; used under {Shown}";
}

public static partial class RustNotices
{
    /// <summary>About's disclosure for the whole list.</summary>
    public static string Heading => $"Rust libraries ({Crates.Count})";
}

/// <summary>What the Windows app ships that is not Inkwell's own.</summary>
public static partial class Notices
{
    /// <summary>The code, in the order About lists it.</summary>
    public static IReadOnlyList<ThirdPartyNotice> Components { get; } =
    [
        new("wasapi-rs", "wasapi-rs, by Henrik Enquist",
            "The way a meeting app's sound is captured: process loopback activation, rewritten against windows-rs.",
            "MIT", WasapiLicence),
        new("llama-cpp", "llama.cpp and ggml, by the ggml authors",
            "Runs the speech model (Qwen3-ASR) on the GPU, through Vulkan, or on the CPU.",
            "MIT", """
MIT License

Copyright (c) 2023-2026 The ggml authors

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
"""),
        new("miniaudio", "miniaudio, by David Reid",
            "Compiled in with llama.cpp's multimodal helper.",
            "Public domain or MIT-0", """
ALTERNATIVE 1 - Public Domain (www.unlicense.org)
===============================================================================
This is free and unencumbered software released into the public domain.

Anyone is free to copy, modify, publish, use, compile, sell, or distribute this
software, either in source code form or as a compiled binary, for any purpose,
commercial or non-commercial, and by any means.

In jurisdictions that recognize copyright laws, the author or authors of this
software dedicate any and all copyright interest in the software to the public
domain. We make this dedication for the benefit of the public at large and to
the detriment of our heirs and successors. We intend this dedication to be an
overt act of relinquishment in perpetuity of all present and future rights to
this software under copyright law.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER LIABILITY, WHETHER IN AN
ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM, OUT OF OR IN CONNECTION
WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE SOFTWARE.

For more information, please refer to <http://unlicense.org/>

===============================================================================
ALTERNATIVE 2 - MIT No Attribution
===============================================================================
Copyright 2026 David Reid

Permission is hereby granted, free of charge, to any person obtaining a copy of
this software and associated documentation files (the "Software"), to deal in
the Software without restriction, including without limitation the rights to
use, copy, modify, merge, publish, distribute, sublicense, and/or sell copies
of the Software, and to permit persons to whom the Software is furnished to do
so.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
"""),
        new("stb-image", "stb_image, by Sean Barrett",
            "Compiled in with llama.cpp's multimodal helper; not called by Inkwell.",
            "MIT", """
ALTERNATIVE A - MIT License
Copyright (c) 2017 Sean Barrett
Permission is hereby granted, free of charge, to any person obtaining a copy of
this software and associated documentation files (the "Software"), to deal in
the Software without restriction, including without limitation the rights to
use, copy, modify, merge, publish, distribute, sublicense, and/or sell copies
of the Software, and to permit persons to whom the Software is furnished to do
so, subject to the following conditions:
The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.
THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
------------------------------------------------------------------------------
ALTERNATIVE B - Public Domain (www.unlicense.org)
This is free and unencumbered software released into the public domain.
Anyone is free to copy, modify, publish, use, compile, sell, or distribute this
software, either in source code form or as a compiled binary, for any purpose,
commercial or non-commercial, and by any means.
In jurisdictions that recognize copyright laws, the author or authors of this
software dedicate any and all copyright interest in the software to the public
domain. We make this dedication for the benefit of the public at large and to
the detriment of our heirs and successors. We intend this dedication to be an
overt act of relinquishment in perpetuity of all present and future rights to
this software under copyright law.
THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER LIABILITY, WHETHER IN AN
ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM, OUT OF OR IN CONNECTION
WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE SOFTWARE.
------------------------------------------------------------------------------
"""),
        new("xxhash", "xxHash, by Yann Collet",
            "Compiled in with llama.cpp.",
            "BSD-2-Clause", """
xxHash Library
Copyright (c) 2012-2021 Yann Collet
All rights reserved.

BSD 2-Clause License (https://www.opensource.org/licenses/bsd-license.php)

Redistribution and use in source and binary forms, with or without modification,
are permitted provided that the following conditions are met:

* Redistributions of source code must retain the above copyright notice, this
  list of conditions and the following disclaimer.

* Redistributions in binary form must reproduce the above copyright notice, this
  list of conditions and the following disclaimer in the documentation and/or
  other materials provided with the distribution.

THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS" AND
ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT LIMITED TO, THE IMPLIED
WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE FOR
ANY DIRECT, INDIRECT, INCIDENTAL, SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES
(INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR SERVICES;
LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON
ANY THEORY OF LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT
(INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE OF THIS
SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.
"""),
        new("sha1", "SHA-1 in C, by Steve Reid",
            "Compiled in with llama.cpp.",
            "Public domain", """
SHA-1 in C
By Steve Reid <steve@edmweb.com>
100% Public Domain
"""),
        new("sha256", "SHA-256, by Igor Pavlov",
            "Compiled in with llama.cpp.",
            "Public domain", """
2010-06-11 : Igor Pavlov : Public domain
"""),
        new("rotate-bits", "rotate-bits, by William Casarin",
            "Compiled in with llama.cpp.",
            "MIT", """
MIT License

Copyright (c) 2021 William Casarin <jb55@jb55.com>

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
"""),
        new("nemo-speech", "NeMo-Speech.cpp, by NVIDIA",
            "Tells the voices on the far end apart (Nemotron-3-Diarization).",
            "Apache-2.0", """
NeMo-Speech.cpp
Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.

This product includes software developed by third parties. See
THIRD_PARTY_NOTICES.md for applicable notices, attributions, and license
terms.

Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.


                                 Apache License
                           Version 2.0, January 2004
                        http://www.apache.org/licenses/

   TERMS AND CONDITIONS FOR USE, REPRODUCTION, AND DISTRIBUTION

   1. Definitions.

      "License" shall mean the terms and conditions for use, reproduction,
      and distribution as defined by Sections 1 through 9 of this document.

      "Licensor" shall mean the copyright owner or entity authorized by
      the copyright owner that is granting the License.

      "Legal Entity" shall mean the union of the acting entity and all
      other entities that control, are controlled by, or are under common
      control with that entity. For the purposes of this definition,
      "control" means (i) the power, direct or indirect, to cause the
      direction or management of such entity, whether by contract or
      otherwise, or (ii) ownership of fifty percent (50%) or more of the
      outstanding shares, or (iii) beneficial ownership of such entity.

      "You" (or "Your") shall mean an individual or Legal Entity
      exercising permissions granted by this License.

      "Source" form shall mean the preferred form for making modifications,
      including but not limited to software source code, documentation
      source, and configuration files.

      "Object" form shall mean any form resulting from mechanical
      transformation or translation of a Source form, including but
      not limited to compiled object code, generated documentation,
      and conversions to other media types.

      "Work" shall mean the work of authorship, whether in Source or
      Object form, made available under the License, as indicated by a
      copyright notice that is included in or attached to the work
      (an example is provided in the Appendix below).

      "Derivative Works" shall mean any work, whether in Source or Object
      form, that is based on (or derived from) the Work and for which the
      editorial revisions, annotations, elaborations, or other modifications
      represent, as a whole, an original work of authorship. For the purposes
      of this License, Derivative Works shall not include works that remain
      separable from, or merely link (or bind by name) to the interfaces of,
      the Work and Derivative Works thereof.

      "Contribution" shall mean any work of authorship, including
      the original version of the Work and any modifications or additions
      to that Work or Derivative Works thereof, that is intentionally
      submitted to Licensor for inclusion in the Work by the copyright owner
      or by an individual or Legal Entity authorized to submit on behalf of
      the copyright owner. For the purposes of this definition, "submitted"
      means any form of electronic, verbal, or written communication sent
      to the Licensor or its representatives, including but not limited to
      communication on electronic mailing lists, source code control systems,
      and issue tracking systems that are managed by, or on behalf of, the
      Licensor for the purpose of discussing and improving the Work, but
      excluding communication that is conspicuously marked or otherwise
      designated in writing by the copyright owner as "Not a Contribution."

      "Contributor" shall mean Licensor and any individual or Legal Entity
      on behalf of whom a Contribution has been received by Licensor and
      subsequently incorporated within the Work.

   2. Grant of Copyright License. Subject to the terms and conditions of
      this License, each Contributor hereby grants to You a perpetual,
      worldwide, non-exclusive, no-charge, royalty-free, irrevocable
      copyright license to reproduce, prepare Derivative Works of,
      publicly display, publicly perform, sublicense, and distribute the
      Work and such Derivative Works in Source or Object form.

   3. Grant of Patent License. Subject to the terms and conditions of
      this License, each Contributor hereby grants to You a perpetual,
      worldwide, non-exclusive, no-charge, royalty-free, irrevocable
      (except as stated in this section) patent license to make, have made,
      use, offer to sell, sell, import, and otherwise transfer the Work,
      where such license applies only to those patent claims licensable
      by such Contributor that are necessarily infringed by their
      Contribution(s) alone or by combination of their Contribution(s)
      with the Work to which such Contribution(s) was submitted. If You
      institute patent litigation against any entity (including a
      cross-claim or counterclaim in a lawsuit) alleging that the Work
      or a Contribution incorporated within the Work constitutes direct
      or contributory patent infringement, then any patent licenses
      granted to You under this License for that Work shall terminate
      as of the date such litigation is filed.

   4. Redistribution. You may reproduce and distribute copies of the
      Work or Derivative Works thereof in any medium, with or without
      modifications, and in Source or Object form, provided that You
      meet the following conditions:

      (a) You must give any other recipients of the Work or
          Derivative Works a copy of this License; and

      (b) You must cause any modified files to carry prominent notices
          stating that You changed the files; and

      (c) You must retain, in the Source form of any Derivative Works
          that You distribute, all copyright, patent, trademark, and
          attribution notices from the Source form of the Work,
          excluding those notices that do not pertain to any part of
          the Derivative Works; and

      (d) If the Work includes a "NOTICE" text file as part of its
          distribution, then any Derivative Works that You distribute must
          include a readable copy of the attribution notices contained
          within such NOTICE file, excluding those notices that do not
          pertain to any part of the Derivative Works, in at least one
          of the following places: within a NOTICE text file distributed
          as part of the Derivative Works; within the Source form or
          documentation, if provided along with the Derivative Works; or,
          within a display generated by the Derivative Works, if and
          wherever such third-party notices normally appear. The contents
          of the NOTICE file are for informational purposes only and
          do not modify the License. You may add Your own attribution
          notices within Derivative Works that You distribute, alongside
          or as an addendum to the NOTICE text from the Work, provided
          that such additional attribution notices cannot be construed
          as modifying the License.

      You may add Your own copyright statement to Your modifications and
      may provide additional or different license terms and conditions
      for use, reproduction, or distribution of Your modifications, or
      for any such Derivative Works as a whole, provided Your use,
      reproduction, and distribution of the Work otherwise complies with
      the conditions stated in this License.

   5. Submission of Contributions. Unless You explicitly state otherwise,
      any Contribution intentionally submitted for inclusion in the Work
      by You to the Licensor shall be under the terms and conditions of
      this License, without any additional terms or conditions.
      Notwithstanding the above, nothing herein shall supersede or modify
      the terms of any separate license agreement you may have executed
      with Licensor regarding such Contributions.

   6. Trademarks. This License does not grant permission to use the trade
      names, trademarks, service marks, or product names of the Licensor,
      except as required for reasonable and customary use in describing the
      origin of the Work and reproducing the content of the NOTICE file.

   7. Disclaimer of Warranty. Unless required by applicable law or
      agreed to in writing, Licensor provides the Work (and each
      Contributor provides its Contributions) on an "AS IS" BASIS,
      WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or
      implied, including, without limitation, any warranties or conditions
      of TITLE, NON-INFRINGEMENT, MERCHANTABILITY, or FITNESS FOR A
      PARTICULAR PURPOSE. You are solely responsible for determining the
      appropriateness of using or redistributing the Work and assume any
      risks associated with Your exercise of permissions under this License.

   8. Limitation of Liability. In no event and under no legal theory,
      whether in tort (including negligence), contract, or otherwise,
      unless required by applicable law (such as deliberate and grossly
      negligent acts) or agreed to in writing, shall any Contributor be
      liable to You for damages, including any direct, indirect, special,
      incidental, or consequential damages of any character arising as a
      result of this License or out of the use or inability to use the
      Work (including but not limited to damages for loss of goodwill,
      work stoppage, computer failure or malfunction, or any and all
      other commercial damages or losses), even if such Contributor
      has been advised of the possibility of such damages.

   9. Accepting Warranty or Additional Liability. While redistributing
      the Work or Derivative Works thereof, You may choose to offer,
      and charge a fee for, acceptance of support, warranty, indemnity,
      or other liability obligations and/or rights consistent with this
      License. However, in accepting such obligations, You may act only
      on Your own behalf and on Your sole responsibility, not on behalf
      of any other Contributor, and only if You agree to indemnify,
      defend, and hold each Contributor harmless for any liability
      incurred by, or claims asserted against, such Contributor by reason
      of your accepting any such warranty or additional liability.

   END OF TERMS AND CONDITIONS

   APPENDIX: How to apply the Apache License to your work.

      To apply the Apache License to your work, attach the following
      boilerplate notice, with the fields enclosed by brackets "[]"
      replaced with your own identifying information. (Don't include
      the brackets!)  The text should be enclosed in the appropriate
      comment syntax for the file format. We also recommend that a
      file or class name and description of purpose be included on the
      same "printed page" as the copyright notice for easier
      identification within third-party archives.

   Copyright [yyyy] [name of copyright owner]

   Licensed under the Apache License, Version 2.0 (the "License");
   you may not use this file except in compliance with the License.
   You may obtain a copy of the License at

       http://www.apache.org/licenses/LICENSE-2.0

   Unless required by applicable law or agreed to in writing, software
   distributed under the License is distributed on an "AS IS" BASIS,
   WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
   See the License for the specific language governing permissions and
   limitations under the License.

NOTICE AND DISCLAIMER: This software automatically retrieves, accesses or
interacts with external materials. Those retrieved materials are not
distributed with this software and are governed solely by separate terms,
conditions and licenses. You are solely responsible for finding, reviewing and
complying with all applicable terms, conditions, and licenses, and for
verifying the security, integrity and suitability of any retrieved materials
for your specific use case. This software is provided "AS IS", without
warranty of any kind. The author makes no representations or warranties
regarding any retrieved materials, damages, liabilities or legal consequences
from your use or inability to use this software or any retrieved materials.
Use this software and the retrieved materials at your own risk.
"""),
        new("nemo-ggml", "ggml, NeMo-Speech.cpp's copy, by the ggml authors",
            "Runs the diarizer, apart from llama.cpp's copy.",
            "MIT", """
MIT License

Copyright (c) 2023-2026 The ggml authors

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
"""),
        new("sentencepiece", "SentencePiece, by Google",
            "Loaded by NeMo-Speech.cpp.",
            "Apache-2.0", Apache2),
        new("protobuf-lite", "protobuf-lite, inside SentencePiece",
            "Part of SentencePiece's library.",
            "BSD-3-Clause", """
Copyright 2008 Google Inc.  All rights reserved.

Redistribution and use in source and binary forms, with or without
modification, are permitted provided that the following conditions are
met:

    * Redistributions of source code must retain the above copyright
notice, this list of conditions and the following disclaimer.
    * Redistributions in binary form must reproduce the above
copyright notice, this list of conditions and the following disclaimer
in the documentation and/or other materials provided with the
distribution.
    * Neither the name of Google Inc. nor the names of its
contributors may be used to endorse or promote products derived from
this software without specific prior written permission.

THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS
"AS IS" AND ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT
LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR
A PARTICULAR PURPOSE ARE DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT
OWNER OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL,
SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT
LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR SERVICES; LOSS OF USE,
DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY
THEORY OF LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT
(INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE
OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.

Code generated by the Protocol Buffer compiler is owned by the owner
of the input file used when generating it.  This code is not
standalone and requires a support library to be linked with it.  This
support library is itself covered by the above license.
""")
        { Composed = true },
        new("darts-clone", "Darts-clone, inside SentencePiece",
            "Part of SentencePiece's library.",
            "BSD-3-Clause", """
Copyright (c) 2008-2011, Susumu Yata
All rights reserved.

Redistribution and use in source and binary forms, with or without modification, are permitted provided that the following conditions are met:

- Redistributions of source code must retain the above copyright notice, this list of conditions and the following disclaimer.
- Redistributions in binary form must reproduce the above copyright notice, this list of conditions and the following disclaimer in the documentation and/or other materials provided with the distribution.
- Neither the name of the <ORGANIZATION> nor the names of its contributors may be used to endorse or promote products derived from this software without specific prior written permission.

THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS" AND ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT OWNER OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL, SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.
""")
        { Composed = true },
        new("abseil", "Abseil, by Google",
            "Loaded by NeMo-Speech.cpp and SentencePiece.",
            "Apache-2.0", Apache2),
        // sherpa-onnx's LICENSE (its 1.13.4 crates.io package, the version of the libraries) is the
        // Apache License exactly, but for a blank first line.
        new("sherpa-onnx", "sherpa-onnx, by the k2-fsa project",
            "Runs Parakeet on the CPU, for the live words and for dictation on a PC without a GPU: sherpa-onnx 1.13.4, copied beside the app.",
            "Apache-2.0", Apache2),
        new("onnxruntime", "ONNX Runtime, by Microsoft",
            "Runs Parakeet's model for sherpa-onnx: ONNX Runtime 1.27.0, copied beside the app.",
            "MIT, with the notices of the code it includes",
            OnnxRuntimeLicence + "\n\n--- Eigen (MPL-2.0): where its source is ---\n" + OnnxRuntimeEigenSource
            + "\n\n--- ThirdPartyNotices.txt ---\n" + OnnxRuntimeNotices)
        { Composed = true },
        // Compiled into sherpa-onnx's library (its symbols and source paths are in the DLL): each
        // its own licence file at the version sherpa-onnx 1.13.4 builds, compared on 2026-09-30
        // (composed-notices.txt).
        new("nlohmann-json", "nlohmann/json, by Niels Lohmann",
            "Part of sherpa-onnx's library: JSON for Modern C++ 3.12.0.",
            "MIT", NlohmannJsonLicence)
        { Composed = true },
        new("kaldi-decoder", "kaldi-decoder, inside sherpa-onnx",
            "Part of sherpa-onnx's library.",
            "Apache-2.0", Apache2)
        { Composed = true },
        new("kaldifst", "kaldifst, inside sherpa-onnx",
            "Part of sherpa-onnx's library.",
            "Apache-2.0", KaldifstLegalNotices + "\n\n" + Apache2)
        { Composed = true },
        new("openfst", "OpenFst, inside sherpa-onnx",
            "Part of sherpa-onnx's library.",
            "Apache-2.0", OpenFstCopying + "\n\n--- The Apache License 2.0, which COPYING names ---\n" + Apache2)
        { Composed = true },
        new("simple-sentencepiece", "simple-sentencepiece, inside sherpa-onnx",
            "Part of sherpa-onnx's library, with its own copy of Darts-clone 0.32 (darts.h).",
            "Apache-2.0, with Darts-clone's BSD-2-Clause notice",
            Apache2 + "\n\n--- ssentencepiece/csrc/darts.h ---\n" + SimpleSentencepieceDartsNotice)
        { Composed = true },
        new("kaldi-native-fbank", "kaldi-native-fbank, inside sherpa-onnx",
            "Part of sherpa-onnx's library: the features Parakeet listens to.",
            "Apache-2.0", Apache2)
        { Composed = true },
        // hclust-cpp's own LICENSE: fastcluster's licence under hclust-cpp's copyright lines (not
        // the Mac's fastcluster text, whose lines are those of FluidAudio's copy).
        new("hclust-cpp", "hclust-cpp's fastcluster, inside sherpa-onnx",
            "Part of sherpa-onnx's library (hierarchical clustering); not called by Inkwell.",
            "BSD-2-Clause", HclustCppFastclusterLicence)
        { Composed = true },
        new("aec3", "aec3, a Rust port of WebRTC AEC3, by Angelos-Ermis Mangos",
            "Cancels the echo of the far end in your microphone.",
            "MIT or BSD-3-Clause, with WebRTC's BSD-3-Clause notice and patent grant", """
# WebRTC Derivative Work License and Patent Grant

This repository contains a Rust port of code derived from the WebRTC project. 
The original WebRTC source code and its derived elements are subject to the license 
and patent grant below.

---

## 1. Copyright for New Contributions

The original code (the Rust implementation) written by Angelos-Ermis Mangos is 
licensed under the MIT License.

Copyright (c) 2025, Angelos-Ermis Mangos. All rights reserved.

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.

## 2. WebRTC Project License and Patent Grant

The following license applies to the portions of this software derived from 
the WebRTC project source code:

Copyright (c) 2011, The WebRTC project authors. All rights reserved.

Redistribution and use in source and binary forms, with or without
modification, are permitted provided that the following conditions are
met:

  * Redistributions of source code must retain the above copyright
    notice, this list of conditions and the following disclaimer.
  * Redistributions in binary form must reproduce the above copyright
    notice, this list of conditions and the following disclaimer in
    the documentation and/or other materials provided with the
    distribution.
  * Neither the name of Google nor the names of its contributors may
    be used to endorse or promote products derived from this software
    without specific prior written permission.

THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS
"AS IS" AND ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT
LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR
A PARTICULAR PURPOSE ARE DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT
HOLDER OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL,
SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT
LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR SERVICES; LOSS OF USE,
DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY
THEORY OF LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT
(INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE
OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.

Additional IP Rights Grant (Patents)
"This implementation" means the copyrightable works distributed by
Google as part of the WebRTC code package.
Google hereby grants to you a perpetual, worldwide, non-exclusive,
no-charge, irrevocable (except as stated in this section) patent
license to make, have made, use, offer to sell, sell, import,
transfer, and otherwise run, modify and propagate the contents of this
implementation of the WebRTC code package, where such license applies
only to those patent claims, both currently owned by Google and
acquired in the future, licensable by Google that are necessarily
infringed by this implementation of the WebRTC code package. This
grant does not include claims that would be infringed only as a
consequence of further modification of this implementation. If you or
your agent or exclusive licensee institute or order or agree to the
institution of patent litigation against any entity (including a
cross-claim or counterclaim in a lawsuit) alleging that this
implementation of the WebRTC code package or any code incorporated
within this implementation of the WebRTC code package constitutes
direct or contributory patent infringement, or inducement of patent
infringement, then any patent rights granted to you under this License
for this implementation of the WebRTC code package shall terminate as
of the date such litigation is filed.
"""),
        new("sqlite", "SQLite",
            "The library's database.",
            "Public domain", """
The author disclaims copyright to this source code. In place of a legal notice, here is a blessing:

    May you do good and not evil.
    May you find forgiveness for yourself and forgive others.
    May you share freely, never taking more than you give.
"""),
        new("webgl-noise", "webgl-noise, by Ian McEwan, Ashima Arts, and Stefan Gustavson",
            "The ink's simplex noise.",
            "MIT", """
Copyright (C) 2011 by Ashima Arts (Simplex noise)
Copyright (C) 2011-2016 by Stefan Gustavson (Classic noise and others)
Permission is hereby granted, free of charge, to any person obtaining a copy of this software and
associated documentation files (the "Software"), to deal in the Software without restriction,
including without limitation the rights to use, copy, modify, merge, publish, distribute, sublicense,
and/or sell copies of the Software, and to permit persons to whom the Software is furnished to do so,
subject to the following conditions:
The above copyright notice and this permission notice shall be included in all copies or substantial
portions of the Software.
THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR IMPLIED, INCLUDING BUT NOT
LIMITED TO THE WARRANTIES OF MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN
NO EVENT SHALL THE AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER LIABILITY,
WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM, OUT OF OR IN CONNECTION WITH THE
SOFTWARE OR THE USE OR OTHER DEALINGS IN THE SOFTWARE.
"""),
        new("windows-app-sdk", "Windows App SDK, by Microsoft",
            "The app's windows and controls (WinUI 3), copied beside the app: Microsoft.WindowsAppSDK.WinUI 2.3.9, Base 2.0.4, Foundation 2.3.12 and InteractiveExperiences 2.1.9.",
            "Microsoft Software License Terms",
            "--- license.txt ---\n" + WindowsAppSdkLicence + "\n" + WindowsAppSdkNotices),
        new("webview2", "WebView2 SDK, by Microsoft",
            "Brought with WinUI 3: Microsoft.Web.WebView2 1.0.3719.77.",
            "BSD-3-Clause",
            "--- LICENSE.txt ---\n" + WebView2Licence + "\n\n--- NOTICE.txt ---\n" + WebView2Notices),
        new("winuiex", "WinUIEx, by Morten Nielsen",
            "The tray icon: WinUIEx 2.9.3.",
            "MIT", WinUIExText)
        { Composed = true },
        new("dotnet-runtime", ".NET runtime",
            "Compiled into Inkwell.exe by NativeAOT: .NET 10.0.12.",
            "MIT, with the notices of the code it includes", DotnetText),
        new("windows-sdk-net", "Windows SDK projection for .NET, by Microsoft",
            "The Windows APIs as C# sees them, shipped with the app: Microsoft.Windows.SDK.NET.dll and WinRT.Runtime.dll from Microsoft.Windows.SDK.NET.Ref 10.0.26100.57.",
            "Windows SDK licence terms", WindowsSdkNetText)
        { Composed = true },
        // C#/WinRT's source is MIT, but the WinRT.Runtime.dll compiled in is Microsoft's build from
        // Microsoft.Windows.SDK.NET.Ref, Distributable Code under the Windows SDK licence (its REDIST
        // list names it): the row says both, and the terms the user agrees to cover it (the
        // windows-sdk-net row, which the first run's terms step shows, names the file).
        new("cswinrt", "C#/WinRT runtime, by Microsoft",
            "How C# calls the Windows APIs, compiled in with the projection: WinRT.Runtime from Microsoft.Windows.SDK.NET.Ref 10.0.26100.57.",
            "MIT; Inkwell's copy under the Windows SDK licence terms", CsWinRtLicence),
        new("velopack", "Velopack, by Velopack Ltd and Caelan Sayler",
            "Installs Inkwell and brings its updates: the installer, Update.exe beside the app, and the update check in Settings > About. Velopack 1.2.161.",
            "MIT", VelopackLicence),
    ];

    /// <summary>
    /// The end-user terms the Windows App SDK's licence asks of an app that ships its runtime
    /// (section 3(b)(ii) of the Microsoft Software License Terms, the windows-app-sdk notice), and
    /// the Windows SDK's licence of an app that ships its .NET projection (Distributable Code,
    /// Distribution Requirements: Microsoft.Windows.SDK.NET.dll and WinRT.Runtime.dll are on its
    /// REDIST list; the windows-sdk-net notice): the user agrees to Microsoft's terms for those
    /// components. The first run asks for that agreement before anything else (TermsStep);
    /// Settings > About shows it above the notices; the installer's splash
    /// (windows/scripts/pack.ps1), the release notes and the download page carry it before
    /// Inkwell first runs.
    /// </summary>
    public const string WindowsAppSdkTerms =
        "Inkwell is free software under the MIT licence. It includes the runtime of Microsoft's Windows App SDK " +
        "and the Windows SDK's .NET projection, which Microsoft licenses separately, under the Microsoft Software " +
        "License Terms shown below (\"Windows App SDK, by Microsoft\" and \"Windows SDK projection for .NET, by Microsoft\"). " +
        "By installing or using Inkwell, you agree to those terms for those components.";

    /// <summary>The ids of the composed notices, which mac/composed-notices.txt lists.</summary>
    public static IReadOnlySet<string> ComposedIds =>
        Components.Where(c => c.Composed).Select(c => c.Id)
            .Concat(Models.Where(m => m.Composed).Select(m => m.Id))
            .ToHashSet();

    /// <summary>The weights, which are downloaded, never bundled.</summary>
    public static IReadOnlyList<ModelCredit> Models { get; } =
    [
        new("qwen3-asr", "Qwen3-ASR 1.7B", "the Qwen team, Alibaba Cloud", "Apache-2.0",
            "Dictation and meeting transcripts.", null),
        // Windows runs Parakeet through sherpa-onnx (S3.2), from an int8 ONNX conversion: the
        // credit says so where the Mac's names its Core ML conversion.
        new("parakeet", "Parakeet TDT 0.6B v3", "NVIDIA", "CC-BY-4.0",
            "The live words while you speak and while a meeting runs.",
            "Parakeet TDT 0.6B v3 by NVIDIA, licensed under the Creative Commons Attribution 4.0 International licence (https://creativecommons.org/licenses/by/4.0/). Converted to ONNX (int8) for sherpa-onnx by csukuangfj on Hugging Face."),
        new("nemotron-diarization", "Nemotron-3-Diarization", "NVIDIA", "OpenMDW-1.1",
            "Who spoke on the far end.", null),
        new("silero-vad", "Silero VAD v6", "the Silero team", "MIT",
            "Hears where speech starts and stops.", Silero)
        { Composed = true },
    ];

    /// <summary>The Apache License 2.0, shared by the components under it (Notices.swift's apache2).</summary>
    public const string Apache2 = """
                                 Apache License
                           Version 2.0, January 2004
                        http://www.apache.org/licenses/

   TERMS AND CONDITIONS FOR USE, REPRODUCTION, AND DISTRIBUTION

   1. Definitions.

      "License" shall mean the terms and conditions for use, reproduction,
      and distribution as defined by Sections 1 through 9 of this document.

      "Licensor" shall mean the copyright owner or entity authorized by
      the copyright owner that is granting the License.

      "Legal Entity" shall mean the union of the acting entity and all
      other entities that control, are controlled by, or are under common
      control with that entity. For the purposes of this definition,
      "control" means (i) the power, direct or indirect, to cause the
      direction or management of such entity, whether by contract or
      otherwise, or (ii) ownership of fifty percent (50%) or more of the
      outstanding shares, or (iii) beneficial ownership of such entity.

      "You" (or "Your") shall mean an individual or Legal Entity
      exercising permissions granted by this License.

      "Source" form shall mean the preferred form for making modifications,
      including but not limited to software source code, documentation
      source, and configuration files.

      "Object" form shall mean any form resulting from mechanical
      transformation or translation of a Source form, including but
      not limited to compiled object code, generated documentation,
      and conversions to other media types.

      "Work" shall mean the work of authorship, whether in Source or
      Object form, made available under the License, as indicated by a
      copyright notice that is included in or attached to the work
      (an example is provided in the Appendix below).

      "Derivative Works" shall mean any work, whether in Source or Object
      form, that is based on (or derived from) the Work and for which the
      editorial revisions, annotations, elaborations, or other modifications
      represent, as a whole, an original work of authorship. For the purposes
      of this License, Derivative Works shall not include works that remain
      separable from, or merely link (or bind by name) to the interfaces of,
      the Work and Derivative Works thereof.

      "Contribution" shall mean any work of authorship, including
      the original version of the Work and any modifications or additions
      to that Work or Derivative Works thereof, that is intentionally
      submitted to Licensor for inclusion in the Work by the copyright owner
      or by an individual or Legal Entity authorized to submit on behalf of
      the copyright owner. For the purposes of this definition, "submitted"
      means any form of electronic, verbal, or written communication sent
      to the Licensor or its representatives, including but not limited to
      communication on electronic mailing lists, source code control systems,
      and issue tracking systems that are managed by, or on behalf of, the
      Licensor for the purpose of discussing and improving the Work, but
      excluding communication that is conspicuously marked or otherwise
      designated in writing by the copyright owner as "Not a Contribution."

      "Contributor" shall mean Licensor and any individual or Legal Entity
      on behalf of whom a Contribution has been received by Licensor and
      subsequently incorporated within the Work.

   2. Grant of Copyright License. Subject to the terms and conditions of
      this License, each Contributor hereby grants to You a perpetual,
      worldwide, non-exclusive, no-charge, royalty-free, irrevocable
      copyright license to reproduce, prepare Derivative Works of,
      publicly display, publicly perform, sublicense, and distribute the
      Work and such Derivative Works in Source or Object form.

   3. Grant of Patent License. Subject to the terms and conditions of
      this License, each Contributor hereby grants to You a perpetual,
      worldwide, non-exclusive, no-charge, royalty-free, irrevocable
      (except as stated in this section) patent license to make, have made,
      use, offer to sell, sell, import, and otherwise transfer the Work,
      where such license applies only to those patent claims licensable
      by such Contributor that are necessarily infringed by their
      Contribution(s) alone or by combination of their Contribution(s)
      with the Work to which such Contribution(s) was submitted. If You
      institute patent litigation against any entity (including a
      cross-claim or counterclaim in a lawsuit) alleging that the Work
      or a Contribution incorporated within the Work constitutes direct
      or contributory patent infringement, then any patent licenses
      granted to You under this License for that Work shall terminate
      as of the date such litigation is filed.

   4. Redistribution. You may reproduce and distribute copies of the
      Work or Derivative Works thereof in any medium, with or without
      modifications, and in Source or Object form, provided that You
      meet the following conditions:

      (a) You must give any other recipients of the Work or
          Derivative Works a copy of this License; and

      (b) You must cause any modified files to carry prominent notices
          stating that You changed the files; and

      (c) You must retain, in the Source form of any Derivative Works
          that You distribute, all copyright, patent, trademark, and
          attribution notices from the Source form of the Work,
          excluding those notices that do not pertain to any part of
          the Derivative Works; and

      (d) If the Work includes a "NOTICE" text file as part of its
          distribution, then any Derivative Works that You distribute must
          include a readable copy of the attribution notices contained
          within such NOTICE file, excluding those notices that do not
          pertain to any part of the Derivative Works, in at least one
          of the following places: within a NOTICE text file distributed
          as part of the Derivative Works; within the Source form or
          documentation, if provided along with the Derivative Works; or,
          within a display generated by the Derivative Works, if and
          wherever such third-party notices normally appear. The contents
          of the NOTICE file are for informational purposes only and
          do not modify the License. You may add Your own attribution
          notices within Derivative Works that You distribute, alongside
          or as an addendum to the NOTICE text from the Work, provided
          that such additional attribution notices cannot be construed
          as modifying the License.

      You may add Your own copyright statement to Your modifications and
      may provide additional or different license terms and conditions
      for use, reproduction, or distribution of Your modifications, or
      for any such Derivative Works as a whole, provided Your use,
      reproduction, and distribution of the Work otherwise complies with
      the conditions stated in this License.

   5. Submission of Contributions. Unless You explicitly state otherwise,
      any Contribution intentionally submitted for inclusion in the Work
      by You to the Licensor shall be under the terms and conditions of
      this License, without any additional terms or conditions.
      Notwithstanding the above, nothing herein shall supersede or modify
      the terms of any separate license agreement you may have executed
      with Licensor regarding such Contributions.

   6. Trademarks. This License does not grant permission to use the trade
      names, trademarks, service marks, or product names of the Licensor,
      except as required for reasonable and customary use in describing the
      origin of the Work and reproducing the content of the NOTICE file.

   7. Disclaimer of Warranty. Unless required by applicable law or
      agreed to in writing, Licensor provides the Work (and each
      Contributor provides its Contributions) on an "AS IS" BASIS,
      WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or
      implied, including, without limitation, any warranties or conditions
      of TITLE, NON-INFRINGEMENT, MERCHANTABILITY, or FITNESS FOR A
      PARTICULAR PURPOSE. You are solely responsible for determining the
      appropriateness of using or redistributing the Work and assume any
      risks associated with Your exercise of permissions under this License.

   8. Limitation of Liability. In no event and under no legal theory,
      whether in tort (including negligence), contract, or otherwise,
      unless required by applicable law (such as deliberate and grossly
      negligent acts) or agreed to in writing, shall any Contributor be
      liable to You for damages, including any direct, indirect, special,
      incidental, or consequential damages of any character arising as a
      result of this License or out of the use or inability to use the
      Work (including but not limited to damages for loss of goodwill,
      work stoppage, computer failure or malfunction, or any and all
      other commercial damages or losses), even if such Contributor
      has been advised of the possibility of such damages.

   9. Accepting Warranty or Additional Liability. While redistributing
      the Work or Derivative Works thereof, You may choose to offer,
      and charge a fee for, acceptance of support, warranty, indemnity,
      or other liability obligations and/or rights consistent with this
      License. However, in accepting such obligations, You may act only
      on Your own behalf and on Your sole responsibility, not on behalf
      of any other Contributor, and only if You agree to indemnify,
      defend, and hold each Contributor harmless for any liability
      incurred by, or claims asserted against, such Contributor by reason
      of your accepting any such warranty or additional liability.

   END OF TERMS AND CONDITIONS

   APPENDIX: How to apply the Apache License to your work.

      To apply the Apache License to your work, attach the following
      boilerplate notice, with the fields enclosed by brackets "[]"
      replaced with your own identifying information. (Don't include
      the brackets!)  The text should be enclosed in the appropriate
      comment syntax for the file format. We also recommend that a
      file or class name and description of purpose be included on the
      same "printed page" as the copyright notice for easier
      identification within third-party archives.

   Copyright [yyyy] [name of copyright owner]

   Licensed under the Apache License, Version 2.0 (the "License");
   you may not use this file except in compliance with the License.
   You may obtain a copy of the License at

       http://www.apache.org/licenses/LICENSE-2.0

   Unless required by applicable law or agreed to in writing, software
   distributed under the License is distributed on an "AS IS" BASIS,
   WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
   See the License for the specific language governing permissions and
   limitations under the License.
""";

    /// <summary>Silero VAD's licence (the standard MIT text with its copyright line; Notices.swift's silero).</summary>
    public const string Silero = """
MIT License

Copyright (c) 2020-present Silero Team

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
""";
}
