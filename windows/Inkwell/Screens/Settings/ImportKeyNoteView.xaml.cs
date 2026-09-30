// The 0.2 key note. The key dictation uses now is the Voice section's (its name, as the dictation
// model gives it), so it comes in as a function and is read each time the note shows.
using Inkwell.Core.Screens;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;

namespace Inkwell.Screens;

public sealed partial class ImportKeyNoteView : UserControl
{
    private readonly ImportNoteModel model;
    private readonly Func<string> currentKeyName;

    /// <param name="currentKeyName">The dictation key's name now (e.g. "Right Ctrl").</param>
    public ImportKeyNoteView(ImportNoteModel importNote, Func<string> currentKeyName)
    {
        model = importNote ?? throw new ArgumentNullException(nameof(importNote));
        this.currentKeyName = currentKeyName ?? throw new ArgumentNullException(nameof(currentKeyName));
        InitializeComponent();
        model.PropertyChanged += (_, _) => Render();
        Render();
    }

    private void OnLoaded(object sender, RoutedEventArgs e)
    {
        model.Load();
        Render();
    }

    private void Render()
    {
        if (model.Note is { } note)
        {
            Note.Text = ImportNoteModel.Text(note, currentKeyName());
            Card.Visibility = Note.Text.Length > 0 ? Visibility.Visible : Visibility.Collapsed;
        }
        else
        {
            Card.Visibility = Visibility.Collapsed;
        }
    }

    private void OnGotIt(object sender, RoutedEventArgs e) => model.Dismiss();
}
