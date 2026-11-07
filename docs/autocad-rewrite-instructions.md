docs/autocad-2026-command-coverage.md
参照上面文件，按照autocad 2026未完成命令继续实施。
更新后命令情况更新到上面的文件中，不要新开。
其中放宽了acad rust约束，允许放开patch， 但需要有入口记录到patch了哪些，方便上游有更新的时候跟踪上游。
保存文件如果是dwg时，默认是另外的文件名，提示用户现在保存dwg功能不完善，为了防止文件破坏，就不覆盖源文件了。
优先实现2D功能，3D优先级下降。
实现的命令，同时需要在UI接线，除非autocad也没有UI入口。
命令的操作逻辑尽量要和autocad一致。
尽量多agent并行开发，增大并发度。
实现了某个功能就做git commit，不要攒大commit
