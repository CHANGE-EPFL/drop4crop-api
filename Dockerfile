FROM python:3.12.3-alpine

ENV POETRY_VERSION=1.6.1
RUN pip install "poetry==$POETRY_VERSION"
ENV PYTHONPATH="$PYTHONPATH:/app"

WORKDIR /app

RUN apk add --no-cache gcc python3-dev geos-dev proj-util proj-dev musl-dev linux-headers gdal-dev build-base
COPY poetry.lock pyproject.toml /app/
RUN poetry config virtualenvs.create false
RUN poetry install --no-interaction --without dev

COPY alembic.ini prestart.sh /app/
COPY migrations /app/migrations
COPY app /app/app

ENTRYPOINT sh prestart.sh
