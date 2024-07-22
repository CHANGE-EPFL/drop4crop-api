from sqlmodel import Field, SQLModel, Column
from typing import Any
from uuid import uuid4, UUID
from geoalchemy2 import Geometry


class Country(SQLModel, table=True):
    iterator: int = Field(
        default=None,
        nullable=False,
        primary_key=True,
        index=True,
    )
    id: UUID = Field(
        default_factory=uuid4,
        index=True,
        nullable=False,
    )

    name: str = Field(
        index=True,
        nullable=False,
    )
    iso_a2: str = Field(
        index=True,
        nullable=False,
    )
    iso_a3: str = Field(
        index=True,
        nullable=False,
    )
    iso_n3: int = Field(
        index=True,
        nullable=False,
    )
    geom: Any = Field(
        default=None, sa_column=Column(Geometry("MULTIPOLYGON", srid=4326))
    )
