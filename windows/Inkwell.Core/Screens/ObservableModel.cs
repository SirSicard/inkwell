// The view models' change signal, as Swift's @Observable is on the Mac. A model calls Changed()
// after it changes; the view re-reads it. PropertyChanged with an empty name means "anything may
// have changed", which x:Bind answers by refreshing every binding to the model. UI thread only,
// like everything the models hold.
using System.ComponentModel;

namespace Inkwell.Core.Screens;

public abstract class ObservableModel : INotifyPropertyChanged
{
    private static readonly PropertyChangedEventArgs Everything = new(string.Empty);

    public event PropertyChangedEventHandler? PropertyChanged;

    /// <summary>Changes so far (tests read it; views never need it).</summary>
    public int Changes { get; private set; }

    /// <summary>Tells the view the model changed.</summary>
    protected void Changed()
    {
        Changes++;
        PropertyChanged?.Invoke(this, Everything);
    }
}
