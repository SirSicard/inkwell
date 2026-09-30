using Xunit;

// InkFrames and the clock are process-wide: tests that count frames or clock threads must not
// share the process with another class drawing at the same time (CI saw 55 frames against one
// surface's 54). The Ink tests run one class at a time.
[assembly: CollectionBehavior(CollectionBehavior.CollectionPerAssembly)]
