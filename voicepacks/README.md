# WIP

## To add a voicepack:

1. fork this repo
2. make a copy of the `TEMPLATE` voicepack in this directory, rename it to the name of your voicepack
3. place all of your ffmpeg-friendly audio files in the `/tracks` subdir
4. in each event category text file (e.g. `kill.txt`), write the names of the audio files to be played randomly. Each
   file name should be on a new line. You can leave the file blank if you don't have audio files for some events.
5. open a PR to the upstream repo. If the CI tests pass, I'll merge and release with your new voicepack.
