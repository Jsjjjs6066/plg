# Playlists with Git (PLG)
Tool for syncing playlists of music files from a remote server with Git.

<p align="center">
    ![Coat of arms of PLG](https://raw.githubusercontent.com/Jsjjjs6066/plg/master/assets/plg.png)
</p>

# Basic usage:
> [!NOTE]
> Keep in mind that quotes are not needed for one word. Quotes my be used even when not needed. Use quotes if you are unsure how your shell will interpret it. 
Intialize a repository and push all files inside it to a remote:
```
plg init --name <PLAYLIST NAME> <REMOTE REPOSITORY>
```
## Example:
```
plg init --name "My playlist" "https://github.com/Jsjjjs6066/playlist"
```
---
Update local playlist, download a song from Youtube (Music) and push it with all other changes to the remote:
```
plg add <LINK TO A YOUTUBE VIDEO/SONG>
```
## Example:
```
plg add "https://www.youtube.com/watch?v=JatTfrdDgn0"
```
---
Update and open the playlist in the default music player:
```
plg play
```
This will skip an update if less than three hours passed from the last update. You can still update by running:
```
plg update
```
You can change the cooldown by using `--update-cooldown-h` and `--update-cooldown-m` options in `plg cfg`.
Set cooldown to 2 hours and 30 minutes:
```
plg cfg --update-cooldown-h 2 --update-cooldown-m 30
```
Set cooldown 30 minutes:
```
plg cfg --update-cooldown-m 30
```
Set cooldown to 2 hours:
```
plg cfg --update-cooldown-h 2
```
Disable the cooldown (always update before playing):
```
plg cfg --update-cooldown-m 0
```
Reset cooldown:
```
plg reset --update-cooldown
```
Disable automatic updating on play:
```
plg cfg --disable-update-on-play true
```
You can reset it by using:
```
plg cfg --disable-update-on-play false
```
or
```
plg reset --disable-update-on-play
```
---
Specify the music player to open the playlist in this time:
```
plg play <PATH TO YOUR PLAYER>
```
## Example:
```
plg play vlc
```
---
Set the default music player:
```
plg cfg --default-player <PATH TO YOUR PLAYER>
```
## Example:
```
plg cfg --default-player vlc
```
---
Download from Youtube (Music) without updating and pushing:
```
plg download <LINK TO A YOUTUBE VIDEO/SONG>
```
## Example:
```
plg download "https://www.youtube.com/watch?v=JatTfrdDgn0"
```
---
Reset all options set using `plg cfg`:
```
plg reset-all
```
---
Generate completions for your shell for easier use:
```
plg completions <SHELL>
```
## Example:
```
plg completions bash
```
